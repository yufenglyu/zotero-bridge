//! Query parsing and search execution (spec section 9).
//!
//! Strategy:
//! - terms with >= 3 characters go through FTS5 `MATCH` with the trigram
//!   tokenizer (substring-friendly for Chinese and mixed text);
//! - 1-2 character terms use `LIKE` post-filtering / a pure LIKE fallback;
//! - field qualifiers (author:, year:, tag:, type:, library:) map to
//!   column-scoped FTS phrases or equality filters;
//! - ranking uses BM25 with the weights from spec section 9.4.

use rusqlite::{params_from_iter, Connection, ToSql};
use zotero_bridge_core::{Error, FieldScope, Result, SearchQuery, SearchResult, Term};

const BM25_WEIGHTS: &str = "10.0, 6.0, 4.0, 3.0, 2.5, 2.0, 0.5, 0.5";

/// Minimum term length (in Unicode chars) usable by the trigram tokenizer.
pub const TRIGRAM_MIN: usize = 3;

/// Parse a raw query string into a `SearchQuery`.
pub fn parse_query(input: &str, default_limit: u32, maximum_limit: u32) -> SearchQuery {
    SearchQuery {
        terms: tokenize(input),
        limit: default_limit.clamp(1, maximum_limit),
    }
}

/// Convenience: parse and immediately build a SearchQuery with explicit limit.
pub fn build_search_query(input: &str, limit: u32) -> SearchQuery {
    SearchQuery {
        terms: tokenize(input),
        limit,
    }
}

fn tokenize(input: &str) -> Vec<Term> {
    let mut terms = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        // Collect one token, honoring double quotes.
        let mut token = String::new();
        let mut quoted = false;
        if c == '"' {
            quoted = true;
            chars.next();
            for ch in chars.by_ref() {
                if ch == '"' {
                    break;
                }
                token.push(ch);
            }
        } else {
            while let Some(&ch) = chars.peek() {
                if ch.is_whitespace() {
                    break;
                }
                token.push(ch);
                chars.next();
            }
        }
        if token.is_empty() {
            continue;
        }
        if quoted {
            terms.push(Term::Phrase(token));
            continue;
        }
        // Field qualifier?
        if let Some((scope, value)) = split_field(&token) {
            if !value.is_empty() {
                terms.push(Term::Field(scope, value.to_string()));
                continue;
            }
        }
        terms.push(Term::Text(token));
    }
    terms
}

fn split_field(token: &str) -> Option<(FieldScope, &str)> {
    let (name, value) = token.split_once(':')?;
    let scope = match name.to_ascii_lowercase().as_str() {
        "author" => FieldScope::Author,
        "year" => FieldScope::Year,
        "tag" => FieldScope::Tag,
        "type" => FieldScope::Type,
        "library" => FieldScope::Library,
        _ => return None,
    };
    Some((scope, value))
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Escape a value for use inside an FTS5 quoted phrase.
fn fts_escape(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// Escape a value for a LIKE pattern with ESCAPE '\'.
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn like_clause(columns: &[&str]) -> String {
    let parts: Vec<String> = columns
        .iter()
        .map(|c| format!("{c} LIKE '%' || ? || '%' ESCAPE '\\'"))
        .collect();
    format!("({})", parts.join(" OR "))
}

const TEXT_LIKE_COLUMNS: &[&str] = &["i.title", "i.creators", "i.primary_creator", "i.tags"];

/// Execute a parsed query against the index.
pub fn search(conn: &Connection, query: &SearchQuery) -> Result<Vec<SearchResult>> {
    let mut fts_clauses: Vec<String> = Vec::new();
    let mut like_conds: Vec<String> = Vec::new();
    let mut eq_conds: Vec<String> = Vec::new();
    // Params must be bound in SQL placeholder order: all LIKE conditions
    // first, then all equality conditions (see SQL assembly below).
    let mut like_params: Vec<Box<dyn ToSql>> = Vec::new();
    let mut eq_params: Vec<Box<dyn ToSql>> = Vec::new();

    // Push one LIKE condition over `columns`, binding the value once per column.
    let push_like = |columns: &[&str],
                     value: &str,
                     like_conds: &mut Vec<String>,
                     like_params: &mut Vec<Box<dyn ToSql>>| {
        like_conds.push(like_clause(columns));
        for _ in columns {
            like_params.push(Box::new(like_escape(value)));
        }
    };

    for term in &query.terms {
        match term {
            Term::Text(t) | Term::Phrase(t) => {
                if char_len(t) >= TRIGRAM_MIN {
                    fts_clauses.push(format!("\"{}\"", fts_escape(t)));
                } else {
                    push_like(TEXT_LIKE_COLUMNS, t, &mut like_conds, &mut like_params);
                }
            }
            Term::Field(scope, value) => match scope {
                FieldScope::Year => {
                    eq_conds.push("i.year = ?".to_string());
                    eq_params.push(Box::new(value.clone()));
                }
                FieldScope::Type => {
                    eq_conds.push("i.item_type = ?".to_string());
                    eq_params.push(Box::new(value.clone()));
                }
                FieldScope::Library => {
                    eq_conds.push("l.display_name LIKE '%' || ? || '%' ESCAPE '\\'".to_string());
                    eq_params.push(Box::new(like_escape(value)));
                }
                FieldScope::Author => {
                    if char_len(value) >= TRIGRAM_MIN {
                        fts_clauses.push(format!(
                            "{{primary_creator creators}} : \"{}\"",
                            fts_escape(value)
                        ));
                    } else {
                        push_like(
                            &["i.creators", "i.primary_creator"],
                            value,
                            &mut like_conds,
                            &mut like_params,
                        );
                    }
                }
                FieldScope::Tag => {
                    if char_len(value) >= TRIGRAM_MIN {
                        fts_clauses.push(format!("tags : \"{}\"", fts_escape(value)));
                    } else {
                        push_like(&["i.tags"], value, &mut like_conds, &mut like_params);
                    }
                }
            },
        }
    }

    let limit = query.limit.max(1) as i64;

    let (sql, match_param): (String, Option<String>) = if !fts_clauses.is_empty() {
        let match_str = fts_clauses.join(" ");
        let mut sql = format!(
            "SELECT i.item_key, l.library_kind, l.zotero_library_id, l.display_name,
                    i.title, i.creators, i.year, i.container_title, i.item_type,
                    i.select_uri,
                    bm25(items_fts, {BM25_WEIGHTS}) AS score
             FROM items_fts
             JOIN items i ON i.id = items_fts.rowid
             JOIN libraries l ON l.id = i.library_id
             WHERE items_fts MATCH ? AND l.enabled = 1"
        );
        for cond in like_conds.iter().chain(eq_conds.iter()) {
            sql.push_str(&format!(" AND {cond}"));
        }
        sql.push_str(" ORDER BY score ASC LIMIT ?");
        (sql, Some(match_str))
    } else if !like_conds.is_empty() || !eq_conds.is_empty() {
        // Pure LIKE fallback for 1-2 character queries (spec section 9.3).
        let mut sql = String::from(
            "SELECT i.item_key, l.library_kind, l.zotero_library_id, l.display_name,
                    i.title, i.creators, i.year, i.container_title, i.item_type,
                    i.select_uri, 0.0 AS score
             FROM items i
             JOIN libraries l ON l.id = i.library_id
             WHERE l.enabled = 1",
        );
        for cond in like_conds.iter().chain(eq_conds.iter()) {
            sql.push_str(&format!(" AND {cond}"));
        }
        sql.push_str(" ORDER BY i.year DESC, i.title ASC LIMIT ?");
        (sql, None)
    } else {
        // Empty query: recent items first.
        let sql = String::from(
            "SELECT i.item_key, l.library_kind, l.zotero_library_id, l.display_name,
                    i.title, i.creators, i.year, i.container_title, i.item_type,
                    i.select_uri, 0.0 AS score
             FROM items i
             JOIN libraries l ON l.id = i.library_id
             WHERE l.enabled = 1
             ORDER BY i.updated_at DESC LIMIT ?",
        );
        (sql, None)
    };

    let mut bound: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(m) = match_param {
        bound.push(Box::new(m));
    }
    bound.extend(like_params);
    bound.extend(eq_params);
    bound.push(Box::new(limit));

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| Error::Query(e.to_string()))?;
    let rows = stmt
        .query_map(params_from_iter(bound.iter().map(|p| p.as_ref())), |row| {
            let kind: String = row.get(1)?;
            Ok(SearchResult {
                item_key: row.get(0)?,
                library_kind: zotero_bridge_core::LibraryKind::parse(&kind)
                    .unwrap_or(zotero_bridge_core::LibraryKind::User),
                zotero_library_id: row.get(2)?,
                display_name: row.get(3)?,
                title: row.get(4)?,
                creators: row.get(5)?,
                year: row.get(6)?,
                container_title: row.get(7)?,
                item_type: row.get(8)?,
                select_uri: row.get(9)?,
                score: row.get(10)?,
            })
        })
        .map_err(|e| Error::Query(e.to_string()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Query(e.to_string()))?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use zotero_bridge_core::models::*;
    use zotero_bridge_core::RemoteLibrary;

    fn fixture_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.upsert_instance(&ServerInfo {
            api_base: "http://localhost:23119/api".into(),
            api_version: Some(3),
            schema_version: Some(50),
            server_id: "test-server".into(),
        })
        .unwrap();
        let lib_id = db
            .upsert_library("test-server", &RemoteLibrary::user())
            .unwrap();

        let item = |key: &str, title: &str, creators: &str, year: &str, tags: &str| IndexedItem {
            library_id: lib_id,
            item_key: key.into(),
            item_version: 1,
            item_type: "journalArticle".into(),
            title: title.into(),
            creators: creators.into(),
            primary_creator: creators.split(';').next().unwrap_or("").trim().to_string(),
            year: year.into(),
            container_title: "Journal of Turbomachinery".into(),
            tags: tags.into(),
            abstract_note: "deep study of rotor dynamics".into(),
            extra: String::new(),
            select_uri: build_select_uri(LibraryKind::User, "0", key),
            content_hash: key.into(),
            ..Default::default()
        };

        let mut db = db;
        db.apply_sync_batch(
            lib_id,
            &SyncBatch {
                upserts: vec![
                    item(
                        "AAAA1111",
                        "燃气轮机转子动力学研究",
                        "张三; 李四",
                        "2024",
                        "仿真",
                    ),
                    item(
                        "BBBB2222",
                        "Turbine blade cooling",
                        "Smith, John",
                        "2023",
                        "turbine",
                    ),
                    item(
                        "CCCC3333",
                        "数字孪生驱动的燃气轮机仿真",
                        "王五",
                        "2024",
                        "数字孪生",
                    ),
                ],
                deleted_keys: vec![],
                mirror_jobs: vec![],
                new_version: 10,
            },
            true,
        )
        .unwrap();
        db
    }

    fn run(db: &Database, q: &str) -> Vec<SearchResult> {
        search(db.connection(), &build_search_query(q, 30)).unwrap()
    }

    #[test]
    fn chinese_substring_match() {
        let db = fixture_db();
        let hits = run(&db, "燃气轮机");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn multi_term_and() {
        let db = fixture_db();
        let hits = run(&db, "燃气轮机 转子");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item_key, "AAAA1111");
    }

    #[test]
    fn english_match() {
        let db = fixture_db();
        let hits = run(&db, "turbine");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].item_key, "BBBB2222");
    }

    #[test]
    fn short_query_like_fallback() {
        let db = fixture_db();
        let hits = run(&db, "李");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item_key, "AAAA1111");
    }

    #[test]
    fn field_filters() {
        let db = fixture_db();
        assert_eq!(run(&db, "author:Smith").len(), 1);
        assert_eq!(run(&db, "year:2024").len(), 2);
        assert_eq!(run(&db, "tag:仿真").len(), 1);
        assert_eq!(run(&db, "type:journalArticle").len(), 3);
        assert_eq!(run(&db, "library:My").len(), 3);
    }

    #[test]
    fn mixed_field_and_short_term_query() {
        // Regression: params must bind in SQL order regardless of the order
        // terms appear in the query string.
        let db = fixture_db();
        assert_eq!(run(&db, "year:2024 转子").len(), 1);
        assert_eq!(run(&db, "转子 year:2024").len(), 1);
        assert!(run(&db, "year:1999 转子").is_empty());
        assert_eq!(run(&db, "tag:仿真 燃气轮机").len(), 1);
        assert_eq!(run(&db, "燃气轮机 tag:仿真").len(), 1);
    }

    #[test]
    fn quoted_phrase() {
        let db = fixture_db();
        let hits = run(&db, "\"blade cooling\"");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn update_removes_old_title_from_search() {
        let mut db = fixture_db();
        let lib_id = db.list_libraries(None).unwrap()[0].id;
        db.apply_sync_batch(
            lib_id,
            &SyncBatch {
                upserts: vec![IndexedItem {
                    library_id: lib_id,
                    item_key: "AAAA1111".into(),
                    item_version: 2,
                    item_type: "journalArticle".into(),
                    title: "航空发动机叶片研究".into(),
                    creators: "张三".into(),
                    primary_creator: "张三".into(),
                    year: "2024".into(),
                    select_uri: build_select_uri(LibraryKind::User, "0", "AAAA1111"),
                    content_hash: "changed".into(),
                    ..Default::default()
                }],
                deleted_keys: vec![],
                mirror_jobs: vec![],
                new_version: 11,
            },
            true,
        )
        .unwrap();
        assert!(run(&db, "转子").is_empty());
        assert_eq!(run(&db, "叶片").len(), 1);
    }

    #[test]
    fn delete_removes_from_search() {
        let mut db = fixture_db();
        let lib_id = db.list_libraries(None).unwrap()[0].id;
        db.apply_sync_batch(
            lib_id,
            &SyncBatch {
                upserts: vec![],
                deleted_keys: vec!["BBBB2222".into()],
                mirror_jobs: vec![],
                new_version: 11,
            },
            true,
        )
        .unwrap();
        assert!(run(&db, "cooling").is_empty());
    }

    #[test]
    fn fts_rebuild_and_integrity() {
        let db = fixture_db();
        db.rebuild_fts().unwrap();
        assert!(db.fts_integrity_check().unwrap());
        assert_eq!(run(&db, "燃气轮机").len(), 2);
    }

    #[test]
    fn injection_attempt_is_safe() {
        let db = fixture_db();
        // Must not error out or return everything.
        let _ = run(&db, "\" OR 1=1 --");
        let _ = run(&db, "'; DROP TABLE items; --");
        assert_eq!(db.count_items().unwrap(), 3);
    }
}
