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
        return Ok(home_dir()?
            .join("Library")
            .join("Logs")
            .join(APP_DIR));
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
        let Some(end_rel) = s[start + 1..].find('%') else { break };
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
}
