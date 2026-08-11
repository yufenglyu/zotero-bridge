//! zotero-bridge-zotero-api: read-only client for the Zotero Local API
//! (http://localhost:23119/api).
//!
//! Only read requests are used. The API requires Zotero to be running and
//! "Allow other applications on this computer to communicate with Zotero"
//! to be enabled. No Web API key is needed.

pub mod client;
pub mod discovery;
pub mod dto;
pub mod source;

pub use client::{LocalApiClient, BATCH_SIZE};
pub use dto::*;
pub use source::ZoteroSource;
