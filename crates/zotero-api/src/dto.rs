//! JSON shapes returned by the Zotero Local API.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `format=versions` response: item key -> version.
pub type VersionsResponse = BTreeMap<String, u64>;

/// Versions arrive as numbers on stable Zotero, but some builds (e.g.
/// Zotero 10.0-beta) return empty strings; treat those as 0.
pub fn de_u64_lenient<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(0),
        serde_json::Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    })
}

/// Convert a raw JSON object from `format=versions` into a version map,
/// tolerating empty-string values.
pub fn parse_versions(body: &str) -> std::result::Result<VersionsResponse, serde_json::Error> {
    let raw: BTreeMap<String, serde_json::Value> = serde_json::from_str(body)?;
    Ok(raw
        .into_iter()
        .map(|(k, v)| {
            let n = match v {
                serde_json::Value::Number(n) => n.as_u64().unwrap_or(0),
                serde_json::Value::String(s) => s.parse().unwrap_or(0),
                _ => 0,
            };
            (k, n)
        })
        .collect())
}

/// A top-level item object from `/items` or `/items/top`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroItem {
    pub key: String,
    #[serde(default, deserialize_with = "de_u64_lenient")]
    pub version: u64,
    #[serde(default)]
    pub data: ItemData,
}

/// The `data` payload of an item. Only fields the indexer needs are
/// modeled; everything else is preserved through `raw_json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ItemData {
    pub item_type: String,
    pub title: String,
    pub creators: Vec<Creator>,
    pub date: String,
    pub publication_title: String,
    pub book_title: String,
    pub proceedings_title: String,
    pub encyclopedia_title: String,
    pub dictionary_title: String,
    pub university: String,
    pub institution: String,
    pub publisher: String,
    pub abstract_note: String,
    pub extra: String,
    pub date_modified: String,
    pub tags: Vec<TagEntry>,
    /// Present (and truthy) when the item is in the trash.
    #[serde(default)]
    pub deleted: serde_json::Value,
    /// Every other Zotero field (edition, number, versionNumber, volume,
    /// pages, DOI, ...) preserved verbatim so `raw_json` keeps the complete
    /// record and the filename templater can read any field.
    #[serde(flatten)]
    pub other: BTreeMap<String, serde_json::Value>,
}

impl ItemData {
    /// Whether the item is in the trash (`deleted: 1` or `deleted: true`).
    pub fn is_trashed(&self) -> bool {
        match &self.deleted {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::Number(n) => n.as_i64().map(|v| v != 0).unwrap_or(false),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Creator {
    pub creator_type: String,
    pub first_name: String,
    pub last_name: String,
    /// Single-field name used by institutional authors.
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TagEntry {
    pub tag: String,
}

/// Response of `GET <prefix>/deleted?since=<version>`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeletedResponse {
    pub collections: Vec<String>,
    pub searches: Vec<String>,
    pub items: Vec<String>,
    pub tags: Vec<String>,
}

/// One group from `GET /api/users/0/groups`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoteroGroup {
    pub id: u64,
    #[serde(default)]
    pub data: GroupData,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupData {
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_item() {
        let json = r#"{
            "key": "N49R8KAQ",
            "version": 315,
            "data": {
                "itemType": "journalArticle",
                "title": "燃气轮机转子动力学研究",
                "creators": [
                    {"creatorType": "author", "firstName": "Wei", "lastName": "Wang"},
                    {"creatorType": "author", "name": "World Health Organization"}
                ],
                "date": "2024-03-01",
                "publicationTitle": "Journal of Turbomachinery",
                "tags": [{"tag": "仿真"}, {"tag": "转子"}],
                "abstractNote": "abstract text",
                "extra": "DOI: 10.1234/abc",
                "dateModified": "2024-03-02T10:00:00Z"
            }
        }"#;
        let item: ZoteroItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.key, "N49R8KAQ");
        assert_eq!(item.version, 315);
        assert_eq!(item.data.item_type, "journalArticle");
        assert_eq!(item.data.creators.len(), 2);
        assert!(!item.data.is_trashed());
    }

    #[test]
    fn parses_trashed_item() {
        let json = r#"{"key":"ABCDEF12","version":1,"data":{"itemType":"book","deleted":1}}"#;
        let item: ZoteroItem = serde_json::from_str(json).unwrap();
        assert!(item.data.is_trashed());
    }

    #[test]
    fn parses_deleted_response() {
        let json = r#"{"collections":[],"searches":[],"items":["N49R8KAQ"],"tags":[]}"#;
        let d: DeletedResponse = serde_json::from_str(json).unwrap();
        assert_eq!(d.items, vec!["N49R8KAQ"]);
    }
}
