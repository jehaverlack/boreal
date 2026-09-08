CREATE TABLE local_file_items (
    id INTEGER PRIMARY KEY,
    root_path TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    name TEXT NOT NULL,
    extension TEXT NOT NULL DEFAULT '',
    is_directory INTEGER NOT NULL DEFAULT 0,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    modified_unix INTEGER NOT NULL DEFAULT 0,
    checksum_sha256 TEXT NOT NULL DEFAULT '',
    is_accessible INTEGER NOT NULL DEFAULT 1,
    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(root_path, relative_path)
);
CREATE INDEX local_file_items_name_idx ON local_file_items(name);
CREATE INDEX local_file_items_size_idx ON local_file_items(size_bytes);
CREATE INDEX local_file_items_checksum_idx ON local_file_items(checksum_sha256);
