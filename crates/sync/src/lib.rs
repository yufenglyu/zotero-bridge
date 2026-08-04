//! zsb-sync: incremental synchronization engine (spec section 12).
//!
//! Flow: probe instance -> discover libraries -> fetch changed versions
//! -> batch-fetch items (max 50 keys) -> fetch deleted keys -> verify
//! library-version stability -> commit one database transaction ->
//! mirror jobs run asynchronously afterwards.

pub mod engine;
pub mod normalizer;
pub mod state;

pub use engine::SyncEngine;
pub use state::{LibrarySyncReport, SyncReport};
