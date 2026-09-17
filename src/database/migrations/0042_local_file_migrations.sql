CREATE TABLE migration_jobs_new (
    id INTEGER PRIMARY KEY,
    source_scope TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('my-drive', 'shared-with-me', 'shared-drive', 'local-files')),
    operation_kind TEXT NOT NULL DEFAULT 'drive-copy' CHECK (operation_kind IN ('drive-copy', 'local-download')),
    status TEXT NOT NULL DEFAULT 'draft',
    phase TEXT NOT NULL DEFAULT 'Select destination',
    destination_kind TEXT NOT NULL DEFAULT 'google-drive' CHECK (destination_kind IN ('google-drive', 'local')),
    destination_url TEXT NOT NULL DEFAULT '',
    destination_path TEXT NOT NULL DEFAULT '',
    destination_drive_id TEXT NOT NULL DEFAULT '',
    destination_drive_name TEXT NOT NULL DEFAULT '',
    destination_folder_id TEXT NOT NULL DEFAULT '',
    destination_folder_name TEXT NOT NULL DEFAULT '',
    files_total INTEGER NOT NULL DEFAULT 0,
    folders_total INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL DEFAULT 0,
    files_copied INTEGER NOT NULL DEFAULT 0,
    bytes_copied INTEGER NOT NULL DEFAULT 0,
    exceptions_count INTEGER NOT NULL DEFAULT 0,
    resume_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at TEXT,
    completed_at TEXT,
    copy_completed_at TEXT,
    archived_at TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    error_message TEXT NOT NULL DEFAULT ''
);

INSERT INTO migration_jobs_new SELECT * FROM migration_jobs;

CREATE TABLE migration_sources_new (
    migration_id INTEGER NOT NULL REFERENCES migration_jobs_new(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    name TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    is_directory INTEGER NOT NULL,
    files_total INTEGER NOT NULL DEFAULT 0,
    folders_total INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending',
    started_at TEXT,
    completed_at TEXT,
    error_message TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (migration_id, item_id)
);

INSERT INTO migration_sources_new
SELECT migration_id, item_id, name, relative_path, is_directory, files_total,
       folders_total, bytes_total, status, started_at, completed_at, error_message
FROM migration_sources;

DROP TABLE migration_sources;
DROP TABLE migration_jobs;
ALTER TABLE migration_jobs_new RENAME TO migration_jobs;
ALTER TABLE migration_sources_new RENAME TO migration_sources;

CREATE INDEX idx_migration_jobs_status ON migration_jobs(status, updated_at DESC);
CREATE INDEX idx_migration_jobs_archived ON migration_jobs(archived_at, created_at DESC);
CREATE INDEX idx_migration_sources_job ON migration_sources(migration_id);

CREATE TABLE local_migration_entries (
    migration_id INTEGER NOT NULL,
    source_id TEXT NOT NULL,
    root_path TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    is_directory INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL,
    PRIMARY KEY(migration_id,source_id,root_path,relative_path),
    FOREIGN KEY(migration_id,source_id) REFERENCES migration_sources(migration_id,item_id) ON DELETE CASCADE
);
