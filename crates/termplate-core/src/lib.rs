//! Termplate core: parsing, the application model, load-time validation,
//! layout solving, field detection, texts, key contexts and the runtime
//! engine with its drawing model. No I/O (spec §16).

pub mod engine;
pub mod error;
pub mod frame;
pub mod menu;
pub mod names;
pub mod protocol;
pub mod host;
pub mod keys;
pub mod layout;
pub mod loader;
pub mod model;
pub mod panel;
pub mod style;
pub mod syntax;
pub mod text;
pub mod token;
pub mod value;

pub use error::{LoadError, LoadErrors};
pub use host::{eval_now, EvalRequest, EvalResults, Evaluated, Expr, ExprKind, Host, JobEvent, JobId, OwnedScope, Outcome, RunRequest, Scope};
pub use model::Model;
pub use value::Value;

use loader::{load_tree, merge, MergeMode, SourceFs};

/// Loads an application: the root file and its includes, then the user's
/// override files (in order), then builds and validates the model. Load
/// warnings end up in [`Model::warnings`].
pub fn load(
    fs: &dyn SourceFs,
    root: &str,
    overrides: &[&str],
    host: &mut dyn Host,
) -> Result<Model, LoadErrors> {
    let (blocks, warnings) = load_tree(fs, root)?;
    build(blocks, warnings, overrides, fs, host)
}

/// Like [`load`], but a root without an `app` block gets one named after
/// the file (spec §2: widget files). The missing version is a warning.
pub fn load_widget(
    fs: &dyn SourceFs,
    root: &str,
    overrides: &[&str],
    host: &mut dyn Host,
) -> Result<Model, LoadErrors> {
    let (mut blocks, mut warnings) = load_tree(fs, root)?;
    if !blocks.iter().any(|b| b.kind == "app") {
        let name = implicit_app_name(root);
        let version = model::SUPPORTED_VERSIONS.last().copied().unwrap_or("0.1");
        let parsed = syntax::parse(root, &format!("app {name}\n- termplate {version}\n"))
            .map_err(LoadErrors)?;
        blocks.splice(0..0, parsed.blocks().cloned());
        warnings.push(LoadError::new(root, 0, format!("no `app` block: using `app {name}` with `- termplate {version}`")));
    }
    build(blocks, warnings, overrides, fs, host)
}

/// The app name of a widget file without an `app` block: the file name in
/// upper case (`ui.termplate` → `UI`, `my-tool.termplate` → `MY_TOOL`).
pub fn implicit_app_name(root: &str) -> String {
    let mut name: String = loader::stem(root)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect();
    while name.contains("__") {
        name = name.replace("__", "_");
    }
    let name = name.trim_matches('_').to_string();
    if names::is_name(&name) { name } else { "WIDGET".to_string() }
}

fn build(
    blocks: Vec<syntax::RawBlock>,
    mut warnings: Vec<LoadError>,
    overrides: &[&str],
    fs: &dyn SourceFs,
    host: &mut dyn Host,
) -> Result<Model, LoadErrors> {
    let (mut blocks, w) = merge(vec![], blocks, MergeMode::Base)?;
    warnings.extend(w);
    for path in overrides {
        let (user, w) = load_tree(fs, path)?;
        warnings.extend(w);
        let (merged, w) = merge(blocks, user, MergeMode::UserOverride)?;
        blocks = merged;
        warnings.extend(w);
    }
    let mut model = Model::build(blocks, host)?;
    warnings.append(&mut model.warnings);
    model.warnings = warnings;
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use host::TestHost;
    use loader::MemFs;

    #[test]
    fn load_applies_user_overrides() {
        let fs = MemFs::new()
            .with("app/app.termplate", "app T\n- termplate 0.1\n\ninclude ui/*\n")
            .with("app/ui/vars.termplate", "vars\n- TARGET text \"/mnt/backup\"\n")
            .with("cfg/overrides/vars.termplate", "vars\n- TARGET text \"/srv\"\n")
            .with("cfg/settings.termplate", "settings\n- language en\n");
        let m = load(&fs, "app/app.termplate", &["cfg/overrides/vars.termplate", "cfg/settings.termplate"], &mut TestHost::new()).unwrap();
        assert_eq!(m.var("TARGET").unwrap().default, Value::Text("/srv".into()));
        assert_eq!(m.settings.language.as_deref(), Some("en"));
    }

    #[test]
    fn widget_file_without_app() {
        let fs = MemFs::new().with("dir/my-ui.termplate", "menu M\n- QUIT\n");
        assert!(load(&fs, "dir/my-ui.termplate", &[], &mut TestHost::new()).is_err(), "`run` needs `app`");
        let m = load_widget(&fs, "dir/my-ui.termplate", &[], &mut TestHost::new()).unwrap();
        assert_eq!(m.app.name, "MY_UI");
        assert_eq!(m.app.version, "0.1");
        assert!(m.warnings.iter().any(|w| w.to_string().contains("no `app` block")));
        assert_eq!(implicit_app_name("7.termplate"), "WIDGET");
    }

    #[test]
    fn widget_file_with_app_keeps_it() {
        let fs = MemFs::new().with("ui.termplate", "app T\n- termplate 0.1\n\nmenu M\n- QUIT\n");
        let m = load_widget(&fs, "ui.termplate", &[], &mut TestHost::new()).unwrap();
        assert_eq!(m.app.name, "T");
        assert!(m.warnings.is_empty());
    }

    #[test]
    fn override_typo_fails_load() {
        let fs = MemFs::new()
            .with("app.termplate", "app T\n- termplate 0.1\n\nvars\n- TARGET text\n")
            .with("o.termplate", "vars\n- TARGTE text\n");
        let e = load(&fs, "app.termplate", &["o.termplate"], &mut TestHost::new()).unwrap_err();
        assert!(e.to_string().contains("cannot patch `TARGTE`"));
    }
}
