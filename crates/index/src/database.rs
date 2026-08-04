//! Database handle: schema, instances, libraries, items, mirror jobs.

use crate::migrations;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use zsb_core::timeutil::now_rfc3339;
use zsb_core::{
    Error, IndexedItem, LibraryState, MirrorJob, MirrorOperation, Platform, RemoteLibrary,
    Result, ServerInfo, SyncBatch,
};

/// A library row joined with its remote identity and sync state.
#[derive(Debug, Clone)]
pub struct LibraryRecord {
    pub id: i64,
    pub remote: RemoteLibrary,
    pub state: LibraryState,
    pub server_id: String,
}

/// Aggregate statistics for `zsb status`.
#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    pub item_count: u64,
    pub library_count: u64,
    pub pending_jobs: u64,
    pub failed_jobs: u64,
    pub last_sync_at: Option<String>,
}

pub struct Database {
    conn: Connection,
    path: Option<PathBuf>,
}

impl Database {
    /// Open (and if needed create) the index database. A corrupt database
    /// file is renamed aside and rebuilt from scratch (spec section 19.6).
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = open_connection(path)?;
        if !quick_check(&conn) {
            drop(conn);
            let backup = path.with_extension(format!(
                "corrupt-{}",
                now_rfc3339().replace([':', '.'], "-")
            ));
            std::fs::rename(path, &backup)?;
            tracing::warn!(
                backup = %backup.display(),
                "database failed quick_check; backed up and recreating"
            );
            let conn = open_connection(path)?;
            let db = Database {
                conn,
                path: Some(path.to_path_buf()),
            };
            db.init()?;
            return Err(Error::DatabaseCorrupt {
                backup: backup.display().to_string(),
            });
        }
        let db = Database {
            conn,
            path: Some(path.to_path_buf()),
        };
        db.init()?;
        Ok(db)
    }

    /// In-memory database for tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(migrations::db_err)?;
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(migrations::db_err)?;
        let db = Database { conn, path: None };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        migrations::run(&self.conn)?;
        Ok(())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    // ------------------------------------------------------------------
    // Instances (spec section 8.1)
    // ------------------------------------------------------------------

    pub fn upsert_instance(&self, info: &ServerInfo) -> Result<()> {
        let now = now_rfc3339();
        self.conn
            .execute(
                "INSERT INTO zotero_instances
                    (server_id, api_base, api_version, schema_version, first_seen_at, last_seen_at, is_active)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, 1)
                 ON CONFLICT(server_id) DO UPDATE SET
                    api_base = excluded.api_base,
                    api_version = excluded.api_version,
                    schema_version = excluded.schema_version,
                    last_seen_at = excluded.last_seen_at",
                params![
                    info.server_id,
                    info.api_base,
                    info.api_version,
                    info.schema_version,
                    now
                ],
            )
            .map_err(migrations::db_err)?;
        Ok(())
    }

    /// Mark exactly one instance active; all others are paused
    /// (spec section 19.3: never mix versions across instances).
    pub fn set_active_instance(&self, server_id: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE zotero_instances SET is_active = (server_id = ?1)",
                params![server_id],
            )
            .map_err(migrations::db_err)?;
        Ok(())
    }

    pub fn active_instance_id(&self) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT server_id FROM zotero_instances WHERE is_active = 1
                 ORDER BY last_seen_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(migrations::db_err)
    }

    // ------------------------------------------------------------------
    // Libraries (spec section 8.2)
    // ------------------------------------------------------------------

    pub fn upsert_library(&self, server_id: &str, remote: &RemoteLibrary) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO libraries
                    (server_id, library_kind, zotero_library_id, display_name, api_prefix)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(server_id, library_kind, zotero_library_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    api_prefix = excluded.api_prefix",
                params![
                    server_id,
                    remote.kind.as_str(),
                    remote.zotero_library_id,
                    remote.display_name,
                    remote.api_prefix
                ],
            )
            .map_err(migrations::db_err)?;
        self.conn
            .query_row(
                "SELECT id FROM libraries
                 WHERE server_id = ?1 AND library_kind = ?2 AND zotero_library_id = ?3",
                params![server_id, remote.kind.as_str(), remote.zotero_library_id],
                |row| row.get(0),
            )
            .map_err(migrations::db_err)
    }

    pub fn list_libraries(&self, server_id: Option<&str>) -> Result<Vec<LibraryRecord>> {
        let mut sql = String::from(
            "SELECT id, server_id, library_kind, zotero_library_id, display_name,
                    api_prefix, last_version, enabled, last_sync_at, last_error
             FROM libraries",
        );
        if server_id.is_some() {
            sql.push_str(" WHERE server_id = ?1");
        }
        sql.push_str(" ORDER BY library_kind, zotero_library_id");
        let mut stmt = self.conn.prepare(&sql).map_err(migrations::db_err)?;
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<LibraryRecord> {
            let kind: String = row.get(2)?;
            let group_id: String = row.get(3)?;
            let id: i64 = row.get(0)?;
            let enabled: bool = row.get(7)?;
            Ok(LibraryRecord {
                id,
                server_id: row.get(1)?,
                remote: RemoteLibrary {
                    kind: zsb_core::LibraryKind::parse(&kind)
                        .unwrap_or(zsb_core::LibraryKind::User),
                    zotero_library_id: group_id,
                    display_name: row.get(4)?,
                    api_prefix: row.get(5)?,
                },
                state: LibraryState {
                    library_id: id,
                    last_version: row.get::<_, i64>(6)? as u64,
                    enabled,
                    last_sync_at: row.get(8)?,
                    last_error: row.get(9)?,
                },
            })
        };
        let rows: Vec<LibraryRecord> = match server_id {
            Some(sid) => stmt
                .query_map(params![sid], map_row)
                .map_err(migrations::db_err)?,
            None => stmt.query_map([], map_row).map_err(migrations::db_err)?,
        }
        .collect::<std::result::Result<_, _>>()
        .map_err(migrations::db_err)?;
        Ok(rows)
    }

    pub fn library_state(&self, library_id: i64) -> Result<LibraryState> {
        self.conn
            .query_row(
                "SELECT id, last_version, enabled, last_sync_at, last_error
                 FROM libraries WHERE id = ?1",
                params![library_id],
                |row| {
                    Ok(LibraryState {
                        library_id: row.get(0)?,
                        last_version: row.get::<_, i64>(1)? as u64,
                        enabled: row.get(2)?,
                        last_sync_at: row.get(3)?,
                        last_error: row.get(4)?,
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    Error::LibraryNotFound(library_id.to_string())
                }
                other => migrations::db_err(other),
            })
    }

    pub fn set_library_enabled(&self, library_id: i64, enabled: bool) -> Result<()> {
        self.conn
            .execute(
                "UPDATE libraries SET enabled = ?2 WHERE id = ?1",
                params![library_id, enabled],
            )
            .map_err(migrations::db_err)?;
        Ok(())
    }

    pub fn record_sync_error(&self, library_id: i64, message: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE libraries SET last_error = ?2 WHERE id = ?1",
                params![library_id, message],
            )
            .map_err(migrations::db_err)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Sync batches (spec section 12.5: one transaction per stable sync)
    // ------------------------------------------------------------------

    pub fn apply_sync_batch(
        &mut self,
        library_id: i64,
        batch: &SyncBatch,
        store_raw_json: bool,
    ) -> Result<()> {
        let now = now_rfc3339();
        let tx = self.conn.transaction().map_err(migrations::db_err)?;

        {
            let mut upsert = tx
                .prepare(
                    "INSERT INTO items (
                        library_id, item_key, item_version, item_type,
                        title, creators, primary_creator, year, container_title,
                        tags, abstract_note, extra,
                        date_modified, select_uri, mirror_filename, content_hash,
                        raw_json, indexed_at, updated_at
                     ) VALUES (
                        :library_id, :item_key, :item_version, :item_type,
                        :title, :creators, :primary_creator, :year, :container_title,
                        :tags, :abstract_note, :extra,
                        :date_modified, :select_uri, :mirror_filename, :content_hash,
                        :raw_json, :now, :now
                     )
                     ON CONFLICT(library_id, item_key) DO UPDATE SET
                        item_version    = excluded.item_version,
                        item_type       = excluded.item_type,
                        title           = excluded.title,
                        creators        = excluded.creators,
                        primary_creator = excluded.primary_creator,
                        year            = excluded.year,
                        container_title = excluded.container_title,
                        tags            = excluded.tags,
                        abstract_note   = excluded.abstract_note,
                        extra           = excluded.extra,
                        date_modified   = excluded.date_modified,
                        select_uri      = excluded.select_uri,
                        mirror_filename = excluded.mirror_filename,
                        content_hash    = excluded.content_hash,
                        raw_json        = excluded.raw_json,
                        updated_at      = excluded.updated_at",
                )
                .map_err(migrations::db_err)?;

            for item in &batch.upserts {
                let raw_json: Option<&str> = if store_raw_json {
                    item.raw_json.as_deref()
                } else {
                    None
                };
                upsert
                    .execute(rusqlite::named_params! {
                        ":library_id": library_id,
                        ":item_key": item.item_key,
                        ":item_version": item.item_version as i64,
                        ":item_type": item.item_type,
                        ":title": item.title,
                        ":creators": item.creators,
                        ":primary_creator": item.primary_creator,
                        ":year": item.year,
                        ":container_title": item.container_title,
                        ":tags": item.tags,
                        ":abstract_note": item.abstract_note,
                        ":extra": item.extra,
                        ":date_modified": item.date_modified,
                        ":select_uri": item.select_uri,
                        ":mirror_filename": item.mirror_filename,
                        ":content_hash": item.content_hash,
                        ":raw_json": raw_json,
                        ":now": now,
                    })
                    .map_err(migrations::db_err)?;
            }
        }

        {
            let mut delete = tx
                .prepare("DELETE FROM items WHERE library_id = ?1 AND item_key = ?2")
                .map_err(migrations::db_err)?;
            for key in &batch.deleted_keys {
                delete
                    .execute(params![library_id, key])
                    .map_err(migrations::db_err)?;
            }
        }

        {
            let mut job = tx
                .prepare(
                    "INSERT INTO mirror_jobs
                        (operation, platform, old_path, new_path, content, status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?6)",
                )
                .map_err(migrations::db_err)?;
            for j in &batch.mirror_jobs {
                job.execute(params![
                    j.operation.as_str(),
                    j.platform.as_str(),
                    j.old_path,
                    j.new_path,
                    j.content,
                    now,
                ])
                .map_err(migrations::db_err)?;
            }
        }

        tx.execute(
            "UPDATE libraries SET last_version = ?2, last_sync_at = ?3, last_error = NULL
             WHERE id = ?1",
            params![library_id, batch.new_version as i64, now],
        )
        .map_err(migrations::db_err)?;

        tx.commit().map_err(migrations::db_err)?;
        Ok(())
    }

    /// Current mirror base filenames for the given item keys, used by the
    /// sync engine to decide between create/rename mirror jobs.
    pub fn mirror_filenames(
        &self,
        library_id: i64,
        keys: &[String],
    ) -> Result<HashMap<String, String>> {
        let mut map = HashMap::new();
        let mut stmt = self
            .conn
            .prepare(
                "SELECT item_key, mirror_filename FROM items
                 WHERE library_id = ?1 AND item_key = ?2 AND mirror_filename IS NOT NULL",
            )
            .map_err(migrations::db_err)?;
        for key in keys {
            let name: Option<String> = stmt
                .query_row(params![library_id, key], |row| row.get(1))
                .optional()
                .map_err(migrations::db_err)?;
            if let Some(name) = name {
                map.insert(key.clone(), name);
            }
        }
        Ok(map)
    }

    /// Look up the select URI for `zsb open --library ... --key ...`.
    pub fn find_select_uri(
        &self,
        library_kind: &str,
        zotero_library_id: &str,
        item_key: &str,
    ) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT i.select_uri FROM items i
                 JOIN libraries l ON l.id = i.library_id
                 WHERE l.library_kind = ?1 AND l.zotero_library_id = ?2 AND i.item_key = ?3",
                params![library_kind, zotero_library_id, item_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(migrations::db_err)
    }

    // ------------------------------------------------------------------
    // FTS maintenance (spec sections 9.2, 19.5)
    // ------------------------------------------------------------------

    pub fn rebuild_fts(&self) -> Result<()> {
        self.conn
            .execute_batch("INSERT INTO items_fts(items_fts) VALUES('rebuild');")
            .map_err(migrations::db_err)
    }

    pub fn optimize_fts(&self) -> Result<()> {
        self.conn
            .execute_batch("INSERT INTO items_fts(items_fts) VALUES('optimize');")
            .map_err(migrations::db_err)
    }

    /// Returns true when the FTS index matches the items table.
    pub fn fts_integrity_check(&self) -> Result<bool> {
        match self.conn.execute_batch(
            "INSERT INTO items_fts(items_fts, rank) VALUES('integrity-check', 1);",
        ) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    // ------------------------------------------------------------------
    // Mirror job outbox (spec section 8.4)
    // ------------------------------------------------------------------

    pub fn pending_jobs(&self, platform: Platform, limit: u32) -> Result<Vec<MirrorJob>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, operation, platform, old_path, new_path, content,
                        status, retry_count, last_error
                 FROM mirror_jobs
                 WHERE status = 'pending' AND platform = ?1
                 ORDER BY id LIMIT ?2",
            )
            .map_err(migrations::db_err)?;
        let rows = stmt
            .query_map(params![platform.as_str(), limit], |row| {
                let op: String = row.get(1)?;
                let platform: String = row.get(2)?;
                Ok(MirrorJob {
                    id: row.get(0)?,
                    operation: MirrorOperation::parse(&op)
                        .unwrap_or(MirrorOperation::Create),
                    platform: match platform.as_str() {
                        "macos" => Platform::Macos,
                        _ => Platform::Windows,
                    },
                    old_path: row.get(3)?,
                    new_path: row.get(4)?,
                    content: row.get(5)?,
                    status: row.get(6)?,
                    retry_count: row.get(7)?,
                    last_error: row.get(8)?,
                })
            })
            .map_err(migrations::db_err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(migrations::db_err)?;
        Ok(rows)
    }

    pub fn complete_job(&self, job_id: i64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE mirror_jobs SET status = 'completed', updated_at = ?2 WHERE id = ?1",
                params![job_id, now_rfc3339()],
            )
            .map_err(migrations::db_err)?;
        Ok(())
    }

    pub fn fail_job(&self, job_id: i64, message: &str, max_retries: u32) -> Result<()> {
        // Exponential backoff is handled by the worker scheduling; here we
        // record the failure and give up after max_retries.
        self.conn
            .execute(
                "UPDATE mirror_jobs
                 SET retry_count = retry_count + 1,
                     last_error = ?2,
                     status = CASE WHEN retry_count + 1 >= ?3 THEN 'failed' ELSE 'pending' END,
                     updated_at = ?4
                 WHERE id = ?1",
                params![job_id, message, max_retries, now_rfc3339()],
            )
            .map_err(migrations::db_err)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Stats / meta
    // ------------------------------------------------------------------

    pub fn stats(&self) -> Result<IndexStats> {
        let item_count: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get::<_, i64>(0))
            .map_err(migrations::db_err)? as u64;
        let library_count: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM libraries", [], |r| r.get::<_, i64>(0))
            .map_err(migrations::db_err)? as u64;
        let pending_jobs: u64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM mirror_jobs WHERE status = 'pending'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(migrations::db_err)? as u64;
        let failed_jobs: u64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM mirror_jobs WHERE status = 'failed'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(migrations::db_err)? as u64;
        let last_sync_at: Option<String> = self
            .conn
            .query_row(
                "SELECT MAX(last_sync_at) FROM libraries",
                [],
                |r| r.get(0),
            )
            .map_err(migrations::db_err)?;
        Ok(IndexStats {
            item_count,
            library_count,
            pending_jobs,
            failed_jobs,
            last_sync_at,
        })
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(migrations::db_err)
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(migrations::db_err)?;
        Ok(())
    }

    /// All item keys currently indexed for a library, used by the
    /// full-scan sync fallback to detect removals.
    pub fn item_keys_for_library(&self, library_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT item_key FROM items WHERE library_id = ?1")
            .map_err(migrations::db_err)?;
        let rows = stmt
            .query_map(params![library_id], |row| row.get(0))
            .map_err(migrations::db_err)?
            .collect::<std::result::Result<Vec<String>, _>>()
            .map_err(migrations::db_err)?;
        Ok(rows)
    }

    /// Whether any indexed item (in any library) has this key; used by
    /// `clean-mirrors` to detect orphaned mirror files.
    pub fn item_key_exists(&self, item_key: &str) -> Result<bool> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE item_key = ?1",
                params![item_key],
                |row| row.get(0),
            )
            .map_err(migrations::db_err)?;
        Ok(count > 0)
    }

    /// Count rows, used by tests and doctor.
    pub fn count_items(&self) -> Result<u64> {
        Ok(self.stats()?.item_count)
    }

    /// Enqueue standalone mirror jobs outside a sync batch (e.g. mirror
    /// cleanup when the Zotero instance changes).
    pub fn enqueue_jobs(&self, jobs: &[zsb_core::NewMirrorJob]) -> Result<()> {
        let now = now_rfc3339();
        let mut stmt = self
            .conn
            .prepare(
                "INSERT INTO mirror_jobs
                    (operation, platform, old_path, new_path, content, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?6)",
            )
            .map_err(migrations::db_err)?;
        for j in jobs {
            stmt.execute(params![
                j.operation.as_str(),
                j.platform.as_str(),
                j.old_path,
                j.new_path,
                j.content,
                now,
            ])
            .map_err(migrations::db_err)?;
        }
        Ok(())
    }

    /// All mirror base filenames belonging to one Zotero instance, used to
    /// clean up mirror files when the instance changes (spec section 19.3).
    pub fn all_mirror_filenames_for_server(&self, server_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT i.mirror_filename FROM items i
                 JOIN libraries l ON l.id = i.library_id
                 WHERE l.server_id = ?1 AND i.mirror_filename IS NOT NULL",
            )
            .map_err(migrations::db_err)?;
        let rows = stmt
            .query_map(params![server_id], |row| row.get(0))
            .map_err(migrations::db_err)?
            .collect::<std::result::Result<Vec<String>, _>>()
            .map_err(migrations::db_err)?;
        Ok(rows)
    }

    /// Fetch a single indexed item (diagnostics and mirror rebuilding).
    pub fn get_item(&self, library_id: i64, item_key: &str) -> Result<Option<IndexedItem>> {
        self.conn
            .query_row(
                "SELECT id, library_id, item_key, item_version, item_type,
                        title, creators, primary_creator, year, container_title,
                        tags, abstract_note, extra, date_modified, select_uri,
                        mirror_filename, content_hash, raw_json
                 FROM items WHERE library_id = ?1 AND item_key = ?2",
                params![library_id, item_key],
                |row| {
                    Ok(IndexedItem {
                        id: row.get(0)?,
                        library_id: row.get(1)?,
                        item_key: row.get(2)?,
                        item_version: row.get::<_, i64>(3)? as u64,
                        item_type: row.get(4)?,
                        title: row.get(5)?,
                        creators: row.get(6)?,
                        primary_creator: row.get(7)?,
                        year: row.get(8)?,
                        container_title: row.get(9)?,
                        tags: row.get(10)?,
                        abstract_note: row.get(11)?,
                        extra: row.get(12)?,
                        date_modified: row.get(13)?,
                        select_uri: row.get(14)?,
                        mirror_filename: row.get(15)?,
                        content_hash: row.get(16)?,
                        raw_json: row.get(17)?,
                    })
                },
            )
            .optional()
            .map_err(migrations::db_err)
    }
}

fn open_connection(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).map_err(migrations::db_err)?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(migrations::db_err)?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(migrations::db_err)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(migrations::db_err)?;
    Ok(conn)
}

fn quick_check(conn: &Connection) -> bool {
    conn.query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0))
        .map(|r| r.eq_ignore_ascii_case("ok"))
        .unwrap_or(false)
}
