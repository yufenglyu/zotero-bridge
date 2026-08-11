//! zotero-bridge-index: local SQLite + FTS5 search index.
//!
//! The index is fully independent from Zotero's own `zotero.sqlite`
//! (spec section 4.3): it stores normalized metadata, sync versions,
//! mirror-job outbox state and the FTS5 trigram search index.

pub mod database;
pub mod migrations;
pub mod search;

pub use database::Database;
pub use search::{build_search_query, parse_query};
