-- Retain existing folder identities and tags while broadening the inventory.
ALTER TABLE keeper_shared_folders ADD COLUMN parent_uid TEXT NOT NULL DEFAULT '';
CREATE INDEX keeper_folder_parent ON keeper_shared_folders(parent_uid);
CREATE TABLE keeper_records (
    record_uid TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    record_type TEXT NOT NULL,
    modified_ms INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 0,
    attachment_count INTEGER NOT NULL DEFAULT 0,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    is_accessible INTEGER NOT NULL DEFAULT 1,
    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE keeper_record_folders (
    record_uid TEXT NOT NULL REFERENCES keeper_records(record_uid) ON DELETE CASCADE,
    folder_uid TEXT NOT NULL REFERENCES keeper_shared_folders(folder_uid) ON DELETE CASCADE,
    PRIMARY KEY(record_uid, folder_uid)
);
CREATE INDEX keeper_record_folder_lookup ON keeper_record_folders(folder_uid,record_uid);
CREATE TABLE keeper_record_tags (
    record_uid TEXT NOT NULL REFERENCES keeper_records(record_uid) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(record_uid, tag_id)
);
