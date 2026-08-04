//! Sync report types.

use zsb_core::LibraryKind;

#[derive(Debug, Clone)]
pub struct LibrarySyncReport {
    pub library_kind: LibraryKind,
    pub zotero_library_id: String,
    pub display_name: String,
    pub upserted: usize,
    pub deleted: usize,
    pub mirror_jobs: usize,
    pub skipped_unchanged: usize,
    pub from_version: u64,
    pub to_version: u64,
    pub full: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub server_id: String,
    pub libraries: Vec<LibrarySyncReport>,
    pub zotero_offline: bool,
}

impl SyncReport {
    pub fn total_changes(&self) -> usize {
        self.libraries
            .iter()
            .map(|l| l.upserted + l.deleted)
            .sum()
    }
}
