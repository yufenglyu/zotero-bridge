//! Data models shared across crates.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Map of item key -> object version as returned by `format=versions`.
pub type VersionMap = BTreeMap<String, u64>;

/// Kind of a Zotero library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryKind {
    User,
    Group,
}

impl LibraryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            LibraryKind::User => "user",
            LibraryKind::Group => "group",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(LibraryKind::User),
            "group" => Some(LibraryKind::Group),
            _ => None,
        }
    }
}

impl fmt::Display for LibraryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A Zotero library as seen through the Local API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteLibrary {
    pub kind: LibraryKind,
    /// "0" for the local user library, numeric group id for groups.
    pub zotero_library_id: String,
    pub display_name: String,
    /// API path prefix, e.g. "/users/0" or "/groups/123456".
    pub api_prefix: String,
}

impl RemoteLibrary {
    pub fn user() -> Self {
        RemoteLibrary {
            kind: LibraryKind::User,
            zotero_library_id: "0".to_string(),
            display_name: "My Library".to_string(),
            api_prefix: "/users/0".to_string(),
        }
    }

    pub fn group(id: impl Into<String>, name: impl Into<String>) -> Self {
        let id = id.into();
        RemoteLibrary {
            kind: LibraryKind::Group,
            api_prefix: format!("/groups/{id}"),
            display_name: name.into(),
            zotero_library_id: id,
        }
    }
}

/// Information about the running Zotero instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub api_base: String,
    pub api_version: Option<u32>,
    pub schema_version: Option<u32>,
    /// Value of the `Zotero-Server-ID` header, or `legacy:<uuid>` for old
    /// Zotero versions that do not send it.
    pub server_id: String,
}

/// Build the `zotero://select/...` URI used to focus an item in Zotero.
///
/// Never derive the group link from the local `libraryID` in
/// `zotero.sqlite`; always use the API group id.
pub fn build_select_uri(kind: LibraryKind, library_id: &str, item_key: &str) -> String {
    match kind {
        LibraryKind::User => format!("zotero://select/library/items/{item_key}"),
        LibraryKind::Group => {
            format!("zotero://select/groups/{library_id}/items/{item_key}")
        }
    }
}

/// A normalized, indexed top-level item. Mirrors the `items` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexedItem {
    pub id: i64,
    pub library_id: i64,
    pub item_key: String,
    pub item_version: u64,
    pub item_type: String,
    pub title: String,
    pub creators: String,
    pub primary_creator: String,
    pub year: String,
    pub container_title: String,
    pub tags: String,
    pub abstract_note: String,
    pub extra: String,
    pub date_modified: Option<String>,
    pub select_uri: String,
    pub mirror_filename: Option<String>,
    pub content_hash: String,
    pub raw_json: Option<String>,
}

/// State of a single library in the local index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryState {
    pub library_id: i64,
    pub last_version: u64,
    pub enabled: bool,
    pub last_sync_at: Option<String>,
    pub last_error: Option<String>,
}

/// A new mirror job to be persisted together with a sync batch.
#[derive(Debug, Clone)]
pub struct NewMirrorJob {
    pub operation: MirrorOperation,
    pub platform: Platform,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub content: Option<String>,
}

/// One batch of changes produced by a sync pass and committed in a single
/// database transaction.
#[derive(Debug, Clone, Default)]
pub struct SyncBatch {
    /// Upserted (new or changed) items.
    pub upserts: Vec<IndexedItem>,
    /// Item keys removed from the index (deleted or trashed).
    pub deleted_keys: Vec<String>,
    /// Filesystem operations to execute asynchronously (outbox pattern).
    pub mirror_jobs: Vec<NewMirrorJob>,
    /// New `Last-Modified-Version` for the library.
    pub new_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Macos,
}

impl Platform {
    pub fn as_str(&self) -> &'static str {
        match self {
            Platform::Windows => "windows",
            Platform::Macos => "macos",
        }
    }

    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Macos
        } else {
            Platform::Windows
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorOperation {
    Create,
    Replace,
    Rename,
    Delete,
}

impl MirrorOperation {
    pub fn as_str(&self) -> &'static str {
        match self {
            MirrorOperation::Create => "create",
            MirrorOperation::Replace => "replace",
            MirrorOperation::Rename => "rename",
            MirrorOperation::Delete => "delete",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "create" => Some(MirrorOperation::Create),
            "replace" => Some(MirrorOperation::Replace),
            "rename" => Some(MirrorOperation::Rename),
            "delete" => Some(MirrorOperation::Delete),
            _ => None,
        }
    }
}

/// A pending filesystem operation (persistent outbox pattern).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorJob {
    pub id: i64,
    pub operation: MirrorOperation,
    pub platform: Platform,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub content: Option<String>,
    pub status: String,
    pub retry_count: u32,
    pub last_error: Option<String>,
}

/// One parsed search term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// Plain free-text term (searched across all FTS columns).
    Text(String),
    /// Quoted phrase.
    Phrase(String),
    /// Field-scoped term, e.g. author:Smith.
    Field(FieldScope, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldScope {
    Author,
    Year,
    Tag,
    Type,
    Library,
}

/// A fully parsed search request.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub terms: Vec<Term>,
    pub limit: u32,
}

/// One search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub item_key: String,
    pub library_kind: LibraryKind,
    pub zotero_library_id: String,
    pub display_name: String,
    pub title: String,
    pub creators: String,
    pub year: String,
    pub container_title: String,
    pub item_type: String,
    pub select_uri: String,
    pub score: f64,
}

impl SearchResult {
    /// `uid` used by Alfred JSON output, e.g. "library:N49R8KAQ".
    pub fn uid(&self) -> String {
        match self.library_kind {
            LibraryKind::User => format!("library:{}", self.item_key),
            LibraryKind::Group => {
                format!("groups/{}:{}", self.zotero_library_id, self.item_key)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_uri_user_library() {
        assert_eq!(
            build_select_uri(LibraryKind::User, "0", "N49R8KAQ"),
            "zotero://select/library/items/N49R8KAQ"
        );
    }

    #[test]
    fn select_uri_group_library() {
        assert_eq!(
            build_select_uri(LibraryKind::Group, "123456", "N49R8KAQ"),
            "zotero://select/groups/123456/items/N49R8KAQ"
        );
    }

    #[test]
    fn remote_library_prefixes() {
        assert_eq!(RemoteLibrary::user().api_prefix, "/users/0");
        let g = RemoteLibrary::group("123456", "Turbomachinery");
        assert_eq!(g.api_prefix, "/groups/123456");
        assert_eq!(g.kind, LibraryKind::Group);
    }
}
