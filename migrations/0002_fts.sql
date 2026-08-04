-- 0002_fts.sql
-- FTS5 full-text search index over items, external content mode
-- (spec section 9). The trigram tokenizer supports substring matching
-- for Chinese, mixed-language titles, authors and identifiers.

CREATE VIRTUAL TABLE items_fts USING fts5(
    title,
    primary_creator,
    creators,
    year,
    container_title,
    tags,
    abstract_note,
    extra,
    content='items',
    content_rowid='id',
    tokenize='trigram case_sensitive 0'
);

CREATE TRIGGER items_ai AFTER INSERT ON items BEGIN
    INSERT INTO items_fts(
        rowid,
        title,
        primary_creator,
        creators,
        year,
        container_title,
        tags,
        abstract_note,
        extra
    )
    VALUES (
        new.id,
        new.title,
        new.primary_creator,
        new.creators,
        new.year,
        new.container_title,
        new.tags,
        new.abstract_note,
        new.extra
    );
END;

CREATE TRIGGER items_ad AFTER DELETE ON items BEGIN
    INSERT INTO items_fts(
        items_fts,
        rowid,
        title,
        primary_creator,
        creators,
        year,
        container_title,
        tags,
        abstract_note,
        extra
    )
    VALUES (
        'delete',
        old.id,
        old.title,
        old.primary_creator,
        old.creators,
        old.year,
        old.container_title,
        old.tags,
        old.abstract_note,
        old.extra
    );
END;

CREATE TRIGGER items_au AFTER UPDATE ON items BEGIN
    INSERT INTO items_fts(
        items_fts,
        rowid,
        title,
        primary_creator,
        creators,
        year,
        container_title,
        tags,
        abstract_note,
        extra
    )
    VALUES (
        'delete',
        old.id,
        old.title,
        old.primary_creator,
        old.creators,
        old.year,
        old.container_title,
        old.tags,
        old.abstract_note,
        old.extra
    );
    INSERT INTO items_fts(
        rowid,
        title,
        primary_creator,
        creators,
        year,
        container_title,
        tags,
        abstract_note,
        extra
    )
    VALUES (
        new.id,
        new.title,
        new.primary_creator,
        new.creators,
        new.year,
        new.container_title,
        new.tags,
        new.abstract_note,
        new.extra
    );
END;
