//! Platform directory layout (spec section 7).
//!
//! Windows:
//!   config:  %APPDATA%\ZoteroSearchBridge\config.toml
//!   data:    %LOCALAPPDATA%\ZoteroSearchBridge\data\index.sqlite
//!   logs:    %LOCALAPPDATA%\ZoteroSearchBridge\logs\
//!   mirrors: %LOCALAPPDATA%\ZoteroSearchBridge\mirrors\windows\
//!
//! macOS:
//!   config + data: ~/Library/Application Support/ZoteroSearchBridge/
//!   logs:          ~/Library/Logs/ZoteroSearchBridge/
//!   mirrors:       ~/Zotero Links/

use crate::errors::{Error, Result};
use std::path::PathBuf;

const APP_DIR: &str = "ZoteroSearchBridge";

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| Error::Config("cannot determine home directory".into()))
}

/// Roaming config root (%APPDATA% on Windows, Application Support on macOS).
fn roaming_root() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Ok(home_dir()?
            .join("Library")
            .join("Application Support")
            .join(APP_DIR))
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join(APP_DIR))
            .ok_or_else(|| Error::Config("APPDATA is not set".into()))
    }
}

/// Local data root (%LOCALAPPDATA% on Windows, Application Support on macOS).
fn local_root() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        roaming_root()
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join(APP_DIR))
            .ok_or_else(|| Error::Config("LOCALAPPDATA is not set".into()))
    }
}

pub fn config_file() -> Result<PathBuf> {
    Ok(roaming_root()?.join("config.toml"))
}

pub fn data_dir() -> Result<PathBuf> {
    Ok(local_root()?.join("data"))
}

pub fn database_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("index.sqlite"))
}

pub fn log_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return Ok(home_dir()?.join("Library").join("Logs").join(APP_DIR));
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(local_root()?.join("logs"))
    }
}

pub fn default_mirror_dir_windows() -> Result<PathBuf> {
    Ok(local_root()?.join("mirrors").join("windows"))
}

pub fn default_mirror_dir_macos() -> Result<PathBuf> {
    Ok(home_dir()?.join("Zotero Links"))
}

/// Expand `%VAR%` (Windows style) and a leading `~` inside a configured path.
pub fn expand_path(input: &str) -> PathBuf {
    let mut s = input.to_string();

    // Expand %VAR% segments.
    while let Some(start) = s.find('%') {
        let Some(end_rel) = s[start + 1..].find('%') else {
            break;
        };
        let end = start + 1 + end_rel;
        let var = &s[start + 1..end];
        let value = std::env::var(var).unwrap_or_default();
        s = format!("{}{}{}", &s[..start], value, &s[end + 1..]);
    }

    // Expand leading ~.
    if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
        if let Ok(home) = home_dir() {
            let rest = &s[1..];
            let rest = rest.trim_start_matches(['/', '\\']);
            return home.join(rest);
        }
    }

    PathBuf::from(s)
}

// ---------------------------------------------------------------------
// Custom-path resolution (env vars / portable mode / config setting)
// ---------------------------------------------------------------------

/// Environment variable overriding the config file location.
pub const ENV_CONFIG: &str = "ZSB_CONFIG";
/// Environment variable overriding the index database location.
pub const ENV_DATABASE: &str = "ZSB_DATABASE";
/// Marker filename enabling portable mode when placed next to the exe.
pub const PORTABLE_CONFIG_NAME: &str = "zsb-config.toml";

/// Directory containing the running executable.
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// Portable-mode marker config next to the executable, if present.
pub fn portable_config_file() -> Option<PathBuf> {
    let candidate = exe_dir()?.join(PORTABLE_CONFIG_NAME);
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

/// Whether the given config path is the portable marker next to the exe.
pub fn is_portable_config(path: &std::path::Path) -> bool {
    portable_config_file().map(|p| p == path).unwrap_or(false)
}

/// Resolve the config file location.
///
/// Priority: explicit override (CLI flag) > `$ZSB_CONFIG` > portable
/// marker `<exe>/zsb-config.toml` > platform default.
pub fn resolve_config_file(cli_override: Option<&std::path::Path>) -> Result<PathBuf> {
    if let Some(p) = cli_override {
        return Ok(p.to_path_buf());
    }
    if let Some(v) = std::env::var_os(ENV_CONFIG) {
        if !v.is_empty() {
            return Ok(PathBuf::from(v));
        }
    }
    if let Some(p) = portable_config_file() {
        return Ok(p);
    }
    config_file()
}

/// Resolve the index database location.
///
/// Priority: explicit override (CLI flag) > `$ZSB_DATABASE` >
/// `[storage].database` in config > portable default `<exe>/data/index.sqlite`
/// (only in portable mode) > platform default.
pub fn resolve_database_file(
    cli_override: Option<&std::path::Path>,
    config_db: Option<&str>,
    portable: bool,
) -> Result<PathBuf> {
    if let Some(p) = cli_override {
        return Ok(p.to_path_buf());
    }
    if let Some(v) = std::env::var_os(ENV_DATABASE) {
        if !v.is_empty() {
            return Ok(PathBuf::from(v));
        }
    }
    if let Some(db) = config_db {
        if !db.trim().is_empty() {
            return Ok(expand_path(db));
        }
    }
    if portable {
        if let Some(dir) = exe_dir() {
            return Ok(dir.join("data").join("index.sqlite"));
        }
    }
    database_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_windows_env_vars() {
        std::env::set_var("ZSB_TEST_DIR", "D:\\Data");
        let p = expand_path("%ZSB_TEST_DIR%\\ZoteroLinks");
        assert_eq!(p, PathBuf::from("D:\\Data\\ZoteroLinks"));
    }

    #[test]
    fn expands_tilde() {
        let p = expand_path("~/Zotero Links");
        assert!(p.ends_with("Zotero Links"));
        assert!(p.is_absolute());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn windows_layout_matches_spec() {
        let cfg = config_file().unwrap().to_string_lossy().into_owned();
        assert!(cfg.ends_with("ZoteroSearchBridge\\config.toml"));
        assert!(cfg.contains("Roaming"));
        let db = database_file().unwrap().to_string_lossy().into_owned();
        assert!(db.ends_with("ZoteroSearchBridge\\data\\index.sqlite"));
        assert!(db.contains("Local"));
    }

    #[test]
    fn database_resolution_cli_wins() {
        let cli = std::path::Path::new("D:\\cli\\a.sqlite");
        let got = resolve_database_file(Some(cli), Some("D:\\cfg\\b.sqlite"), true).unwrap();
        assert_eq!(got, cli.to_path_buf());
    }

    #[test]
    fn database_resolution_config_wins_over_portable() {
        std::env::set_var("ZSB_TEST_DB", "D:\\custom");
        let got = resolve_database_file(None, Some("%ZSB_TEST_DB%\\index.sqlite"), true).unwrap();
        assert_eq!(got, PathBuf::from("D:\\custom\\index.sqlite"));
    }

    #[test]
    fn database_resolution_portable_default() {
        let got = resolve_database_file(None, None, true).unwrap();
        let expect = exe_dir().unwrap().join("data").join("index.sqlite");
        assert_eq!(got, expect);
    }

    #[test]
    fn database_resolution_platform_default() {
        let got = resolve_database_file(None, Some(""), false).unwrap();
        assert_eq!(got, database_file().unwrap());
        let got = resolve_database_file(None, None, false).unwrap();
        assert_eq!(got, database_file().unwrap());
    }

    #[test]
    fn config_resolution_cli_wins() {
        let cli = std::path::Path::new("D:\\cli\\config.toml");
        let got = resolve_config_file(Some(cli)).unwrap();
        assert_eq!(got, cli.to_path_buf());
    }
}
