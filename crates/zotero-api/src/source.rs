//! Abstract Zotero data source (spec section 17.1).
//!
//! The sync engine only talks to this trait, so tests can substitute a
//! mock source and a future direct-sqlite diagnostic reader could
//! implement the same interface.

use crate::dto::{DeletedResponse, ZoteroItem};
use std::future::Future;
use zotero_bridge_core::{RemoteLibrary, Result, ServerInfo, VersionMap};

/// Versions of changed items plus the library version that produced them.
#[derive(Debug, Clone, Default)]
pub struct VersionResponse {
    pub versions: VersionMap,
    pub last_modified_version: u64,
}

/// Fully fetched items plus the library version that produced them.
#[derive(Debug, Clone, Default)]
pub struct ItemResponse {
    pub items: Vec<ZoteroItem>,
    pub last_modified_version: u64,
}

/// Deleted object keys plus the library version that produced them.
#[derive(Debug, Clone, Default)]
pub struct DeletedObjects {
    pub deleted: DeletedResponse,
    pub last_modified_version: u64,
}

/// Read-only access to a Zotero instance.
pub trait ZoteroSource: Send + Sync {
    fn probe(&self) -> impl Future<Output = Result<ServerInfo>> + Send;

    fn list_libraries(&self) -> impl Future<Output = Result<Vec<RemoteLibrary>>> + Send;

    fn changed_item_versions(
        &self,
        library: &RemoteLibrary,
        since: u64,
    ) -> impl Future<Output = Result<VersionResponse>> + Send;

    fn fetch_items(
        &self,
        library: &RemoteLibrary,
        keys: &[String],
    ) -> impl Future<Output = Result<ItemResponse>> + Send;

    fn deleted_objects(
        &self,
        library: &RemoteLibrary,
        since: u64,
    ) -> impl Future<Output = Result<DeletedObjects>> + Send;
}
