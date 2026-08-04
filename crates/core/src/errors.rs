//! Error types shared by every crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("zotero is not reachable: {0}")]
    ZoteroOffline(String),

    #[error("zotero local API is disabled (403); enable it in Zotero: Settings -> Advanced -> Allow other applications on this computer to communicate with Zotero")]
    ApiDisabled,

    #[error("zotero server instance changed (412 Precondition Failed)")]
    InstanceChanged,

    #[error("zotero API error: status {status} {message}")]
    Api { status: u16, message: String },

    #[error("http error: {0}")]
    Http(String),

    #[error("invalid json: {0}")]
    Json(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("database is corrupt and was backed up to {backup}; a fresh database was created")]
    DatabaseCorrupt { backup: String },

    #[error("search query error: {0}")]
    Query(String),

    #[error("mirror error: {0}")]
    Mirror(String),

    #[error("library not found: {0}")]
    LibraryNotFound(String),

    #[error("item not found: {0}")]
    ItemNotFound(String),

    #[error("launcher error: {0}")]
    Launcher(String),
}
