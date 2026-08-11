//! Schema migrations, tracked with `PRAGMA user_version`.

use rusqlite::Connection;
use zotero_bridge_core::{Error, Result};

const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../../../migrations/0001_initial.sql")),
    (2, include_str!("../../../migrations/0002_fts.sql")),
];

pub fn run(conn: &Connection) -> Result<()> {
    let current: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db_err)?;
    for (version, sql) in MIGRATIONS {
        if *version > current {
            conn.execute_batch(sql).map_err(db_err)?;
            conn.pragma_update(None, "user_version", *version)
                .map_err(db_err)?;
            tracing::info!(version, "applied database migration");
        }
    }
    Ok(())
}

pub fn current_version(conn: &Connection) -> Result<u32> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db_err)
}

pub(crate) fn db_err(e: rusqlite::Error) -> Error {
    Error::Database(e.to_string())
}
