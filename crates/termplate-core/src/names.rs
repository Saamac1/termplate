//! Names and namespaces (RLML D 2.1, D 4).
//!
//! Names are written in upper case (`MAIN_MENU`, `LOOT.RARE`); lower case is
//! vocabulary. A file's `namespace` prefixes the names it defines, and a
//! reference is looked up in the file's namespace, then in each outer one,
//! then in the root. `.NAME` refers to the root directly.

/// One name part: `A–Z` first, then `A–Z 0–9 _`, no leading, trailing or
/// doubled underscore.
fn is_part(p: &str) -> bool {
    let mut chars = p.chars();
    matches!(chars.next(), Some('A'..='Z'))
        && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !p.ends_with('_')
        && !p.contains("__")
}

/// `MAIN_MENU`, `LOOT.RARE`.
pub fn is_name(s: &str) -> bool {
    !s.is_empty() && s.split('.').all(is_part)
}

/// A name whose trailing parts may be lower-case vocabulary, like a tile
/// variant (`DOOR.open`) or a layout variant (`MAIN.narrow`).
pub fn is_name_with_variant(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    let upper = parts.iter().take_while(|p| is_part(p)).count();
    upper > 0
        && parts[upper..]
            .iter()
            .all(|p| p.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
}

/// The full name of a definition `name` written in namespace `ns`.
pub fn qualify(ns: Option<&str>, name: &str) -> String {
    match (ns, name.strip_prefix('.')) {
        (_, Some(root)) => root.to_string(),
        (Some(ns), None) => format!("{ns}.{name}"),
        (None, None) => name.to_string(),
    }
}

/// Candidate full names for a reference, in lookup order (D 4.4).
pub fn candidates(ns: Option<&str>, name: &str) -> Vec<String> {
    if let Some(root) = name.strip_prefix('.') {
        return vec![root.to_string()];
    }
    let mut out = Vec::new();
    if let Some(ns) = ns {
        let parts: Vec<&str> = ns.split('.').collect();
        for n in (1..=parts.len()).rev() {
            out.push(format!("{}.{name}", parts[..n].join(".")));
        }
    }
    out.push(name.to_string());
    out
}

/// Resolves a reference; the error lists every name tried.
pub fn resolve(ns: Option<&str>, name: &str, exists: impl Fn(&str) -> bool) -> Result<String, String> {
    let tried = candidates(ns, name);
    tried.iter().find(|c| exists(c)).cloned().ok_or_else(|| tried.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_format() {
        for ok in ["A", "MAIN_MENU", "LOOT.RARE", "SFX.HIT2", "A1_B"] {
            assert!(is_name(ok), "{ok}");
        }
        for bad in ["", "lower", "Mixed", "_A", "A_", "A__B", "3D_FIRE", "A..B", "A.", ".A", "A-B", "ÄÄ"] {
            assert!(!is_name(bad), "{bad}");
        }
    }

    #[test]
    fn variant_names() {
        assert!(is_name_with_variant("MAIN"));
        assert!(is_name_with_variant("MAIN.narrow"));
        assert!(is_name_with_variant("UI.MAIN.narrow_tall"));
        assert!(!is_name_with_variant("narrow"));
        assert!(!is_name_with_variant("MAIN.narrow.WIDE"));
        assert!(!is_name_with_variant("MAIN.Narrow"));
    }

    #[test]
    fn qualifying() {
        assert_eq!(qualify(Some("UNDEAD"), "LOOT"), "UNDEAD.LOOT");
        assert_eq!(qualify(None, "LOOT"), "LOOT");
        assert_eq!(qualify(Some("UNDEAD"), ".LOOT"), "LOOT");
    }

    #[test]
    fn lookup_order() {
        assert_eq!(candidates(Some("A.B"), "N"), ["A.B.N", "A.N", "N"]);
        assert_eq!(candidates(None, "N"), ["N"]);
        assert_eq!(candidates(Some("A.B"), ".N"), ["N"]);
        assert_eq!(candidates(Some("UNDEAD"), "SFX.HIT"), ["UNDEAD.SFX.HIT", "SFX.HIT"]);
    }

    #[test]
    fn resolution() {
        let defined = ["A.N", "N", "M"];
        let exists = |n: &str| defined.contains(&n);
        assert_eq!(resolve(Some("A.B"), "N", exists), Ok("A.N".into()), "inner shadows outer");
        assert_eq!(resolve(Some("A.B"), ".N", exists), Ok("N".into()));
        assert_eq!(resolve(Some("A"), "M", exists), Ok("M".into()));
        assert_eq!(resolve(Some("A.B"), "X", exists), Err("A.B.X, A.X, X".into()));
    }
}
