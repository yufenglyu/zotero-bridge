//! Normalization of raw Zotero items into indexed records (spec section 10).

use regex::Regex;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;
use zsb_core::{build_select_uri, IndexedItem, RemoteLibrary};
use zsb_zotero_api::{Creator, ZoteroItem};

/// Item types excluded from the index by default (spec section 10.1).
pub const EXCLUDED_TYPES: &[&str] = &["attachment", "note", "annotation"];

fn year_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(1[5-9]\d{2}|20\d{2}|21\d{2})\b").unwrap())
}

/// Unicode NFKC + control-character removal + whitespace collapsing
/// (spec section 10.6).
pub fn clean_text(input: &str) -> String {
    let normalized: String = input.nfkc().collect();
    let without_controls: String = normalized
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut out = String::with_capacity(without_controls.len());
    let mut last_was_space = true;
    for c in without_controls.chars() {
        if c.is_whitespace() {
            if !last_was_space {
                out.push(' ');
            }
            last_was_space = true;
        } else {
            out.push(c);
            last_was_space = false;
        }
    }
    out.trim().to_string()
}

/// One creator rendered for display, e.g. "Wang, Wei" or an institution name.
pub fn format_creator(c: &Creator) -> String {
    if !c.name.is_empty() {
        return clean_text(&c.name);
    }
    let last = clean_text(&c.last_name);
    let first = clean_text(&c.first_name);
    match (last.is_empty(), first.is_empty()) {
        (false, false) => format!("{last}, {first}"),
        (false, true) => last,
        (true, false) => first,
        (true, true) => String::new(),
    }
}

/// All creators joined with "; " (spec section 10.3).
pub fn format_creators(creators: &[Creator]) -> String {
    creators
        .iter()
        .map(format_creator)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Primary creator: first author, else first editor, else first creator,
/// else "无作者" (spec section 10.3).
pub fn pick_primary_creator(creators: &[Creator]) -> String {
    let by_type = |t: &str| {
        creators
            .iter()
            .find(|c| c.creator_type.eq_ignore_ascii_case(t))
            .map(format_creator)
            .filter(|s| !s.is_empty())
    };
    by_type("author")
        .or_else(|| by_type("editor"))
        .or_else(|| {
            creators
                .first()
                .map(format_creator)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "无作者".to_string())
}

/// First plausible 4-digit year in a free-form date string.
pub fn extract_year(date: &str) -> String {
    year_regex()
        .find(date)
        .map(|m| m.as_str().to_string())
        .unwrap_or_default()
}

/// Merge the publication-related fields by priority (spec section 10.5).
pub fn container_title(data: &zsb_zotero_api::ItemData) -> String {
    for candidate in [
        &data.publication_title,
        &data.book_title,
        &data.proceedings_title,
        &data.encyclopedia_title,
        &data.dictionary_title,
        &data.university,
        &data.institution,
        &data.publisher,
    ] {
        let cleaned = clean_text(candidate);
        if !cleaned.is_empty() {
            return cleaned;
        }
    }
    String::new()
}

/// Hash of the normalized searchable fields; used to skip no-op updates.
pub fn content_hash(item: &IndexedItem) -> String {
    let mut hasher = Sha256::new();
    for field in [
        &item.title,
        &item.creators,
        &item.primary_creator,
        &item.year,
        &item.container_title,
        &item.tags,
        &item.abstract_note,
        &item.extra,
        &item.item_type,
    ] {
        hasher.update(field.as_bytes());
        hasher.update([0x1f]);
    }
    format!("{:x}", hasher.finalize())
}

/// Normalize a raw API item. Returns `None` for excluded types and for
/// trashed items (the caller treats those as deletions).
pub fn normalize_item(
    raw: &ZoteroItem,
    library: &RemoteLibrary,
    library_db_id: i64,
    store_raw_json: bool,
    index_abstract: bool,
    index_extra: bool,
) -> Option<IndexedItem> {
    if EXCLUDED_TYPES.contains(&raw.data.item_type.as_str()) || raw.data.is_trashed() {
        return None;
    }

    let title = {
        let t = clean_text(&raw.data.title);
        if t.is_empty() {
            format!("[无标题] -- {}", raw.key)
        } else {
            t
        }
    };
    let creators = format_creators(&raw.data.creators);
    let primary_creator = pick_primary_creator(&raw.data.creators);
    let year = extract_year(&raw.data.date);
    let container = container_title(&raw.data);
    let tags = raw
        .data
        .tags
        .iter()
        .map(|t| clean_text(&t.tag))
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    let abstract_note = if index_abstract {
        clean_text(&raw.data.abstract_note)
    } else {
        String::new()
    };
    let extra = if index_extra {
        clean_text(&raw.data.extra)
    } else {
        String::new()
    };
    let date_modified = {
        let d = clean_text(&raw.data.date_modified);
        if d.is_empty() {
            None
        } else {
            Some(d)
        }
    };

    let mut item = IndexedItem {
        library_id: library_db_id,
        item_key: raw.key.clone(),
        item_version: raw.version,
        item_type: raw.data.item_type.clone(),
        title,
        creators,
        primary_creator,
        year,
        container_title: container,
        tags,
        abstract_note,
        extra,
        date_modified,
        select_uri: build_select_uri(library.kind, &library.zotero_library_id, &raw.key),
        content_hash: String::new(),
        raw_json: if store_raw_json {
            serde_json::to_string(raw).ok()
        } else {
            None
        },
        ..Default::default()
    };
    item.content_hash = content_hash(&item);
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zsb_core::RemoteLibrary;

    #[test]
    fn year_extraction() {
        assert_eq!(extract_year("2024-03-01"), "2024");
        assert_eq!(extract_year("March 1999"), "1999");
        assert_eq!(extract_year("[2023]"), "2023");
        assert_eq!(extract_year("no date"), "");
        assert_eq!(extract_year("1492"), "");
    }

    #[test]
    fn creator_formatting() {
        let c = Creator {
            creator_type: "author".into(),
            first_name: "Wei".into(),
            last_name: "Wang".into(),
            name: String::new(),
        };
        assert_eq!(format_creator(&c), "Wang, Wei");
        let org = Creator {
            creator_type: "author".into(),
            name: "World Health Organization".into(),
            ..Default::default()
        };
        assert_eq!(format_creator(&org), "World Health Organization");
    }

    #[test]
    fn primary_creator_priority() {
        let editor = Creator {
            creator_type: "editor".into(),
            first_name: "Ed".into(),
            last_name: "Itor".into(),
            ..Default::default()
        };
        let author = Creator {
            creator_type: "author".into(),
            first_name: "Au".into(),
            last_name: "Thor".into(),
            ..Default::default()
        };
        assert_eq!(
            pick_primary_creator(&[editor.clone(), author.clone()]),
            "Thor, Au"
        );
        assert_eq!(pick_primary_creator(&[editor]), "Itor, Ed");
        assert_eq!(pick_primary_creator(&[]), "无作者");
    }

    #[test]
    fn text_cleaning() {
        assert_eq!(clean_text("  hello \n\t world  "), "hello world");
        assert_eq!(clean_text("ｆｕｌｌｗｉｄｔｈ"), "fullwidth");
        assert_eq!(clean_text("a\u{0007}b"), "a b");
    }

    #[test]
    fn filters_excluded_types() {
        let raw: ZoteroItem =
            serde_json::from_str(r#"{"key":"K1","version":1,"data":{"itemType":"attachment"}}"#)
                .unwrap();
        assert!(normalize_item(&raw, &RemoteLibrary::user(), 1, true, true, true).is_none());
    }

    #[test]
    fn filters_trashed_items() {
        let raw: ZoteroItem = serde_json::from_str(
            r#"{"key":"K1","version":1,"data":{"itemType":"book","deleted":1}}"#,
        )
        .unwrap();
        assert!(normalize_item(&raw, &RemoteLibrary::user(), 1, true, true, true).is_none());
    }

    #[test]
    fn empty_title_gets_placeholder() {
        let raw: ZoteroItem =
            serde_json::from_str(r#"{"key":"K1ABCD","version":1,"data":{"itemType":"book"}}"#)
                .unwrap();
        let item = normalize_item(&raw, &RemoteLibrary::user(), 1, true, true, true).unwrap();
        assert_eq!(item.title, "[无标题] -- K1ABCD");
    }

    #[test]
    fn group_select_uri() {
        let raw: ZoteroItem = serde_json::from_str(
            r#"{"key":"K1","version":1,"data":{"itemType":"book","title":"T"}}"#,
        )
        .unwrap();
        let group = RemoteLibrary::group("123456", "G");
        let item = normalize_item(&raw, &group, 2, false, true, true).unwrap();
        assert_eq!(item.select_uri, "zotero://select/groups/123456/items/K1");
        assert!(item.raw_json.is_none());
    }

    #[test]
    fn content_hash_changes_with_content() {
        let raw: ZoteroItem = serde_json::from_str(
            r#"{"key":"K1","version":1,"data":{"itemType":"book","title":"A"}}"#,
        )
        .unwrap();
        let a = normalize_item(&raw, &RemoteLibrary::user(), 1, true, true, true).unwrap();
        let raw2: ZoteroItem = serde_json::from_str(
            r#"{"key":"K1","version":2,"data":{"itemType":"book","title":"B"}}"#,
        )
        .unwrap();
        let b = normalize_item(&raw2, &RemoteLibrary::user(), 1, true, true, true).unwrap();
        assert_ne!(a.content_hash, b.content_hash);
    }
}
