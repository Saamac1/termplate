//! File system access for loading, and the user configuration directory
//! (spec §13).

use std::path::{Path, PathBuf};

use termplate_core::loader::{glob_match, has_wildcard, SourceFs};

pub struct DiskFs;

impl SourceFs for DiskFs {
    fn read(&self, path: &str) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))
    }

    fn glob(&self, pattern: &str) -> Result<Vec<String>, String> {
        if !has_wildcard(pattern) {
            return Ok(if Path::new(pattern).is_file() { vec![pattern.to_string()] } else { vec![] });
        }
        let absolute = pattern.starts_with('/');
        let mut current: Vec<String> = vec![if absolute { "/".into() } else { String::new() }];
        let parts: Vec<&str> = pattern.split('/').filter(|p| !p.is_empty()).collect();
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let mut next = Vec::new();
            for base in &current {
                let join = |name: &str| if base.is_empty() { name.to_string() } else if base.ends_with('/') { format!("{base}{name}") } else { format!("{base}/{name}") };
                if !has_wildcard(part) {
                    let p = join(part);
                    let exists = if last { Path::new(&p).is_file() } else { Path::new(&p).is_dir() };
                    if exists {
                        next.push(p);
                    }
                    continue;
                }
                let dir = if base.is_empty() { ".".to_string() } else { base.clone() };
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') || !glob_match(part, &name) {
                        continue;
                    }
                    let p = join(&name);
                    let ok = if last { Path::new(&p).is_file() } else { Path::new(&p).is_dir() };
                    if ok {
                        next.push(p);
                    }
                }
            }
            current = next;
        }
        current.sort();
        Ok(current)
    }
}

/// The executable `name` (a path, or a name looked up in `path`).
pub fn find_program(name: &str, path: Option<&str>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if name.contains('/') {
        let p = PathBuf::from(name);
        return executable(&p).then_some(p);
    }
    path?.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join(name)).find(|p| executable(p))
}

/// `s` as one word for `sh`: in single quotes.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `$XDG_CONFIG_HOME/termplate/APP`, falling back to `~/.config`.
pub fn config_dir(app: &str, xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg.filter(|x| !x.is_empty() && x.starts_with('/')) {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(home.filter(|h| !h.is_empty())?).join(".config"),
    };
    Some(base.join("termplate").join(app))
}

/// The user's override files for `app`, in load order: everything in
/// `overrides/` (sorted), then `settings.termplate`.
pub fn override_files(dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(dir.join("overrides"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "termplate" || e == "rlml"))
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let settings = dir.join("settings.termplate");
    if settings.is_file() {
        files.push(settings.to_string_lossy().into_owned());
    }
    files
}

/// Completions of a typed path (spec §8): entries of the typed directory
/// whose names start with the typed name. Relative paths are read from
/// `workdir`, `~/` from `home`. Hidden entries only when the typed name
/// starts with `.`. Results keep the typed directory part; directories end
/// in `/`.
pub fn complete_path(prefix: &str, workdir: Option<&str>, home: Option<&str>) -> Vec<String> {
    let (dir_part, name) = match prefix.rfind('/') {
        Some(i) => (&prefix[..=i], &prefix[i + 1..]),
        None => ("", prefix),
    };
    let mut dir = match (dir_part.strip_prefix("~/"), home) {
        (Some(rest), Some(h)) => Path::new(h).join(rest),
        _ => PathBuf::from(if dir_part.is_empty() { "." } else { dir_part }),
    };
    if dir.is_relative()
        && let Some(w) = workdir {
            dir = Path::new(w).join(dir);
        }
    let Ok(entries) = std::fs::read_dir(&dir) else { return vec![] };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            if !n.starts_with(name) || (n.starts_with('.') && !name.starts_with('.')) {
                return None;
            }
            let is_dir = e.path().is_dir();
            Some(format!("{dir_part}{n}{}", if is_dir { "/" } else { "" }))
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("termplate-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn glob_on_disk() {
        let d = tmpdir("glob");
        std::fs::create_dir_all(d.join("ui/sub")).unwrap();
        for f in ["ui/b.termplate", "ui/a.termplate", "ui/.hidden.termplate", "ui/sub/c.termplate", "ui/notes.txt"] {
            std::fs::write(d.join(f), "").unwrap();
        }
        let base = d.to_string_lossy();
        let found = DiskFs.glob(&format!("{base}/ui/*.termplate")).unwrap();
        let names: Vec<&str> = found.iter().map(|p| p.rsplit('/').next().unwrap()).collect();
        assert_eq!(names, ["a.termplate", "b.termplate"]);
        assert_eq!(DiskFs.glob(&format!("{base}/*/sub/*")).unwrap().len(), 1);
        assert_eq!(DiskFs.glob(&format!("{base}/ui/a.termplate")).unwrap().len(), 1);
        assert!(DiskFs.glob(&format!("{base}/nope/*")).unwrap().is_empty());
        assert!(DiskFs.read(&format!("{base}/missing")).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn config_dir_resolution() {
        assert_eq!(config_dir("APP", Some("/xdg"), Some("/home/u")), Some(PathBuf::from("/xdg/termplate/APP")));
        assert_eq!(config_dir("APP", None, Some("/home/u")), Some(PathBuf::from("/home/u/.config/termplate/APP")));
        assert_eq!(config_dir("APP", Some("relative"), Some("/h")), Some(PathBuf::from("/h/.config/termplate/APP")), "XDG must be absolute");
        assert_eq!(config_dir("APP", None, None), None);
    }

    #[test]
    fn override_file_order() {
        let d = tmpdir("ovr");
        std::fs::create_dir_all(d.join("overrides")).unwrap();
        std::fs::write(d.join("overrides/keys.termplate"), "").unwrap();
        std::fs::write(d.join("overrides/a.termplate"), "").unwrap();
        std::fs::write(d.join("overrides/readme.md"), "").unwrap();
        std::fs::write(d.join("settings.termplate"), "").unwrap();
        let files = override_files(&d);
        let names: Vec<&str> = files.iter().map(|p| p.rsplit('/').next().unwrap()).collect();
        assert_eq!(names, ["a.termplate", "keys.termplate", "settings.termplate"]);
        assert!(override_files(&d.join("missing")).is_empty());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn path_completion() {
        let d = std::env::temp_dir().join(format!("termplate-complete-{}", std::process::id()));
        std::fs::create_dir_all(d.join("music")).unwrap();
        std::fs::create_dir_all(d.join("movies")).unwrap();
        std::fs::write(d.join("memo.txt"), "").unwrap();
        std::fs::write(d.join(".mrc"), "").unwrap();
        let w = d.to_str().unwrap();
        assert_eq!(complete_path("m", Some(w), None), ["memo.txt", "movies/", "music/"]);
        assert_eq!(complete_path("mu", Some(w), None), ["music/"]);
        assert_eq!(complete_path(".m", Some(w), None), [".mrc"]);
        assert_eq!(complete_path("~/mo", None, Some(w)), ["~/movies/"]);
        let abs = format!("{w}/me");
        assert_eq!(complete_path(&abs, None, None), [format!("{w}/memo.txt")]);
        assert!(complete_path("nothing/x", Some(w), None).is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
