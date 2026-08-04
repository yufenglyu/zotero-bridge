//! Application configuration (spec section 18).

use crate::errors::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub app: AppConfig,
    pub zotero: ZoteroConfig,
    pub search: SearchConfig,
    pub mirror: MirrorConfig,
    pub maintenance: MaintenanceConfig,
    pub storage: StorageConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub poll_interval_seconds: u64,
    pub start_at_login: bool,
    pub log_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoteroConfig {
    pub api_base: String,
    pub request_timeout_seconds: u64,
    pub include_user_library: bool,
    /// "all" or "none" (group allow-listing is a future extension).
    pub group_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub default_limit: u32,
    pub maximum_limit: u32,
    pub index_abstract: bool,
    pub index_extra: bool,
    pub store_raw_json: bool,
    pub short_query_fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MirrorConfig {
    pub windows: MirrorPlatformConfig,
    pub macos: MirrorPlatformConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MirrorPlatformConfig {
    pub enabled: bool,
    pub directory: String,
    pub template: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Custom index database location. Empty = default (portable data dir
    /// next to the exe in portable mode, otherwise the platform data dir).
    /// Supports %VAR% and ~ expansion.
    pub database: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MaintenanceConfig {
    pub optimize_after_updates: u64,
    pub retain_logs_days: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            poll_interval_seconds: 15,
            start_at_login: true,
            log_level: "info".into(),
        }
    }
}

impl Default for ZoteroConfig {
    fn default() -> Self {
        ZoteroConfig {
            api_base: "http://localhost:23119/api".into(),
            request_timeout_seconds: 10,
            include_user_library: true,
            group_mode: "all".into(),
        }
    }
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            default_limit: 30,
            maximum_limit: 100,
            index_abstract: true,
            index_extra: true,
            store_raw_json: true,
            short_query_fallback: true,
        }
    }
}

pub const DEFAULT_TEMPLATE: &str = "{primary_creator} - {year} - {title} -- {item_key}";

impl Default for MirrorConfig {
    fn default() -> Self {
        MirrorConfig {
            windows: MirrorPlatformConfig {
                enabled: true,
                directory: crate::paths::default_mirror_dir_windows()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "%LOCALAPPDATA%/ZoteroSearchBridge/mirrors/windows".into()),
                template: DEFAULT_TEMPLATE.into(),
            },
            macos: MirrorPlatformConfig {
                enabled: false,
                directory: "~/Zotero Links".into(),
                template: DEFAULT_TEMPLATE.into(),
            },
        }
    }
}

impl Default for MirrorPlatformConfig {
    fn default() -> Self {
        MirrorPlatformConfig {
            enabled: false,
            directory: String::new(),
            template: DEFAULT_TEMPLATE.into(),
        }
    }
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        MaintenanceConfig {
            optimize_after_updates: 5000,
            retain_logs_days: 14,
        }
    }
}

impl Config {
    pub fn mirror_for(&self, platform: crate::models::Platform) -> &MirrorPlatformConfig {
        match platform {
            crate::models::Platform::Windows => &self.mirror.windows,
            crate::models::Platform::Macos => &self.mirror.macos,
        }
    }

    pub fn mirror_dir(&self, platform: crate::models::Platform) -> PathBuf {
        crate::paths::expand_path(&self.mirror_for(platform).directory)
    }
}

/// Load the config file, creating it with defaults when missing.
pub fn load_or_create(path: &Path) -> Result<Config> {
    if !path.exists() {
        let cfg = Config::default();
        save(path, &cfg)?;
        return Ok(cfg);
    }
    let text = fs::read_to_string(path)?;
    let cfg: Config =
        toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    Ok(cfg)
}

/// Save atomically: write to a temporary file, then replace (spec section 18).
pub fn save(path: &Path, cfg: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text =
        toml::to_string_pretty(cfg).map_err(|e| Error::Config(format!("serialize config: {e}")))?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrip() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.app.poll_interval_seconds, 15);
        assert_eq!(parsed.zotero.api_base, "http://localhost:23119/api");
        assert_eq!(parsed.search.default_limit, 30);
        assert!(parsed.mirror.windows.template.contains("{item_key}"));
    }

    #[test]
    fn load_missing_creates_default() {
        let dir = std::env::temp_dir().join(format!("zsb-cfg-{}", uuid_v4()));
        let path = dir.join("config.toml");
        let cfg = load_or_create(&path).unwrap();
        assert!(path.exists());
        assert_eq!(cfg.search.maximum_limit, 100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn uuid_v4() -> String {
        uuid::Uuid::new_v4().to_string()
    }
}
