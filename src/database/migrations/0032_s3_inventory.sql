CREATE TABLE s3_objects (
    id INTEGER PRIMARY KEY,
    remote_name TEXT NOT NULL,
    object_path TEXT NOT NULL,
    name TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    modified_at TEXT NOT NULL DEFAULT '',
    is_directory INTEGER NOT NULL DEFAULT 0,
    mime_type TEXT NOT NULL DEFAULT '',
    checksum TEXT NOT NULL DEFAULT '',
    is_accessible INTEGER NOT NULL DEFAULT 1,
    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(remote_name, object_path)
);
CREATE INDEX s3_objects_name_idx ON s3_objects(name);
CREATE INDEX s3_objects_size_idx ON s3_objects(size_bytes);
