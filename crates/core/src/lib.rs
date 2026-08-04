//! zsb-core: shared configuration, data models and error types for
//! Zotero Search Bridge.

pub mod config;
pub mod errors;
pub mod models;
pub mod paths;
pub mod timeutil;

pub use config::Config;
pub use errors::{Error, Result};
pub use models::*;
