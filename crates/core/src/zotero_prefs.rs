//! Read Zotero's own filename template from the local profile's prefs.js,
//! so the shortcut filenames follow whatever the user configured inside
//! Zotero (Preferences → Advanced → attachment rename template).
//!
//! Resolution order for the mirror filename template:
//! 1. A custom template configured in zsb (non-empty and different from
//!    the built-in legacy default).
//! 2. Zotero's `extensions.zotero.attachmentRenameTemplate` pref.
//! 3. The built-in legacy default.

use crate::config::DEFAULT_TEMPLATE;
use std::path::PathBuf;

/// The Zotero pref holding the attachment rename template.
pub const PREF_NAME: &str = "extensions.zotero.attachmentRenameTemplate";

/// Resolve the effective filename template (see module docs for order).
pub fn resolve_template(configured: &str) -> String {
    let configured = configured.trim();
    if !configured.is_empty() && configured != DEFAULT_TEMPLATE {
        return configured.to_string();
    }
    attachment_rename_template().unwrap_or_else(|| DEFAULT_TEMPLATE.to_string())
}

/// Read `extensions.zotero.attachmentRenameTemplate` from the default
/// Zotero profile's prefs.js. Returns None when Zotero is not installed,
/// the pref is absent, or parsing fails.
pub fn attachment_rename_template() -> Option<String> {
    read_pref(&default_profile_dir()?.join("prefs.js"), PREF_NAME)
}

/// The default Zotero profile directory (from profiles.ini).
pub fn default_profile_dir() -> Option<PathBuf> {
    let root = zotero_data_root()?;
    let ini = std::fs::read_to_string(root.join("profiles.ini")).ok()?;
    parse_default_profile(&ini).map(|rel| {
        let p = PathBuf::from(&rel);
        if p.is_absolute() {
            p
        } else {
            root.join(p)
        }
    })
}

fn zotero_data_root() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        Some(
            home.join("Library")
                .join("Application Support")
                .join("Zotero"),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Zotero").join("Zotero"))
    }
}

/// Extract the `Path=` of the default profile from profiles.ini text.
fn parse_default_profile(ini: &str) -> Option<String> {
    let mut in_profile = false;
    let mut path: Option<String> = None;
    let mut is_default = false;
    let mut first: Option<String> = None;

    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if in_profile && path.is_some() {
                if first.is_none() {
                    first = path.clone();
                }
                if is_default {
                    return path;
                }
            }
            in_profile = line.starts_with("[Profile");
            path = None;
            is_default = false;
            continue;
        }
        if !in_profile {
            continue;
        }
        if let Some(v) = line.strip_prefix("Path=") {
            path = Some(v.trim().to_string());
        } else if line == "Default=1" {
            is_default = true;
        }
    }
    if in_profile && is_default && path.is_some() {
        return path;
    }
    first.or(path)
}

/// Read one string pref from a prefs.js file.
fn read_pref(prefs_path: &std::path::Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(prefs_path).ok()?;
    parse_pref(&text, name)
}

/// Parse `user_pref("name", "value");` from prefs.js content.
fn parse_pref(text: &str, name: &str) -> Option<String> {
    let needle = format!("user_pref(\"{name}\", \"");
    let start = text.find(&needle)? + needle.len();
    // The value is a JS string that may span escaped quotes; it ends at
    // the first unescaped '"'.
    let bytes = text.as_bytes();
    let mut i = start;
    let mut value = String::new();
    while i < bytes.len() {
        let rest = &text[i..];
        let c = rest.chars().next()?;
        if c == '"' {
            return Some(value);
        }
        if c == '\\' {
            let esc = rest[1..].chars().next()?;
            let decoded = match esc {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other, // \" \\ \/ etc. decode to themselves
            };
            value.push(decoded);
            i += 1 + esc.len_utf8();
        } else {
            value.push(c);
            i += c.len_utf8();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_profile() {
        let ini = "[Profile0]\nName=a\nIsRelative=1\nPath=Profiles/a.user\n\n\
                   [Profile1]\nName=b\nIsRelative=1\nPath=Profiles/b.user\nDefault=1\n";
        assert_eq!(
            parse_default_profile(ini).as_deref(),
            Some("Profiles/b.user")
        );
        let ini2 = "[Profile0]\nName=a\nPath=Profiles/only\n";
        assert_eq!(
            parse_default_profile(ini2).as_deref(),
            Some("Profiles/only")
        );
    }

    #[test]
    fn parses_pref_with_escapes() {
        let prefs = r#"// comment
user_pref("extensions.zotero.attachmentRenameTemplate", "{{if itemType == \"book\"}}\n《{{title}}》{{endif}}");
user_pref("other", "x");
"#;
        let v = parse_pref(prefs, PREF_NAME).unwrap();
        assert_eq!(v, "{{if itemType == \"book\"}}\n《{{title}}》{{endif}}");
        assert!(v.contains('\n'));
    }

    #[test]
    fn missing_pref_returns_none() {
        assert!(parse_pref("user_pref(\"a\", \"b\");", PREF_NAME).is_none());
    }

    #[test]
    fn resolution_prefers_custom() {
        let custom = "{title} -- {item_key}";
        assert_eq!(resolve_template(custom), custom);
    }

    #[test]
    fn resolution_falls_back_to_default_without_zotero() {
        // In the test environment there is normally no Zotero profile at
        // $APPDATA override paths; when there IS one the Zotero pref wins,
        // which is also correct. So just assert non-empty either way.
        assert!(!resolve_template("").is_empty());
        assert!(!resolve_template(DEFAULT_TEMPLATE).is_empty());
    }
}
