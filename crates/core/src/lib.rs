//! zsb-core: shared configuration, data models and error types for
//! Zotero Bridge.

pub mod config;
pub mod errors;
pub mod models;
pub mod paths;
pub mod timeutil;
pub mod zotero_prefs;

pub use config::Config;
pub use errors::{Error, Result};
pub use models::*;
