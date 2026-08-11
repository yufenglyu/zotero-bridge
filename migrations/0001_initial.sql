-- 0001_initial.sql
-- Core tables for Zotero Bridge.

CREATE TABLE zotero_instances (
    server_id       TEXT PRIMARY KEY,
    api_base        TEXT NOT NULL,
    api_version     INTEGER,
    schema_version  INTEGER,
    first_seen_at   TEXT NOT NULL,
    last_seen_at    TEXT NOT NULL,
    is_active       INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE libraries (
    id                  INTEGER PRIMARY KEY,
    server_id           TEXT NOT NULL,
    library_kind        TEXT NOT NULL,
    zotero_library_id   TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    api_prefix          TEXT NOT NULL,
    last_version        INTEGER NOT NULL DEFAULT 0,
    enabled             INTEGER NOT NULL DEFAULT 1,
    last_sync_at        TEXT,
    last_error          TEXT,

    FOREIGN KEY(server_id)
        REFERENCES zotero_instances(server_id),

    UNIQUE(server_id, library_kind, zotero_library_id)
);

CREATE TABLE items (
    id                  INTEGER PRIMARY KEY,
    library_id          INTEGER NOT NULL,
    item_key            TEXT NOT NULL,
    item_version        INTEGER NOT NULL,
    item_type           TEXT NOT NULL,

    title               TEXT NOT NULL DEFAULT '',
    creators            TEXT NOT NULL DEFAULT '',
    primary_creator     TEXT NOT NULL DEFAULT '',
    year                TEXT NOT NULL DEFAULT '',
    container_title     TEXT NOT NULL DEFAULT '',
    tags                TEXT NOT NULL DEFAULT '',
    abstract_note       TEXT NOT NULL DEFAULT '',
    extra               TEXT NOT NULL DEFAULT '',

    date_modified       TEXT,
    select_uri          TEXT NOT NULL,
    mirror_filename     TEXT,
    content_hash        TEXT NOT NULL,
    raw_json            TEXT,

    indexed_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,

    FOREIGN KEY(library_id)
        REFERENCES libraries(id)
        ON DELETE CASCADE,

    UNIQUE(library_id, item_key)
);

CREATE INDEX idx_items_library ON items(library_id);
CREATE INDEX idx_items_year ON items(year);

CREATE TABLE mirror_jobs (
    id              INTEGER PRIMARY KEY,
    operation       TEXT NOT NULL,
    platform        TEXT NOT NULL,
    old_path        TEXT,
    new_path        TEXT,
    content         TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    retry_count     INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX idx_mirror_jobs_status ON mirror_jobs(status, platform);

-- Small key/value store for legacy server ids, maintenance counters, etc.
CREATE TABLE meta (
    key     TEXT PRIMARY KEY,
    value   TEXT NOT NULL
);
