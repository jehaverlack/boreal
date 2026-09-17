ALTER TABLE local_file_items ADD COLUMN modified_nanos INTEGER;
ALTER TABLE local_file_items ADD COLUMN file_identity TEXT NOT NULL DEFAULT '';
CREATE TABLE local_file_snapshots (
    id INTEGER PRIMARY KEY,
    completed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    scope TEXT NOT NULL,
    item_count INTEGER NOT NULL
);
CREATE TABLE local_file_snapshot_items (
    snapshot_id INTEGER NOT NULL REFERENCES local_file_snapshots(id) ON DELETE CASCADE,
    root_path TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    metadata TEXT NOT NULL,
    PRIMARY KEY(snapshot_id, root_path, relative_path)
);

-- Repair older directory-entry sizes using the indexed descendants only.
UPDATE local_file_items AS folder
SET size_bytes = COALESCE((
    SELECT SUM(file.size_bytes) FROM local_file_items AS file
    WHERE file.root_path = folder.root_path AND file.is_accessible = 1
      AND file.is_directory = 0 AND file.is_symlink = 0
      AND file.relative_path >= folder.relative_path || '/'
      AND file.relative_path < folder.relative_path || '0'
      AND substr(file.relative_path, 1, length(folder.relative_path) + 1) = folder.relative_path || '/'
), 0)
WHERE folder.is_directory = 1;

-- Keep the existing index as a baseline on upgrade, including an empty index.
INSERT INTO local_file_snapshots(completed_at,scope,item_count)
SELECT value, '', (SELECT COUNT(*) FROM local_file_items WHERE is_accessible=1)
FROM settings WHERE key='local_files.last_sync_at';
INSERT INTO local_file_snapshot_items(snapshot_id,root_path,relative_path,metadata)
SELECT (SELECT id FROM local_file_snapshots LIMIT 1),root_path,relative_path,
    json_object('root_path',root_path,'relative_path',relative_path,'name',name,
        'extension',extension,'is_directory',json(CASE WHEN is_directory=1 THEN 'true' ELSE 'false' END),
        'size_bytes',size_bytes,'modified_unix',modified_unix,'checksum_sha256',checksum_sha256,
        'is_symlink',json(CASE WHEN is_symlink=1 THEN 'true' ELSE 'false' END),'symlink_target',symlink_target,
        'owner_username',owner_username,'owner_identifier',owner_identifier,
        'group_name',group_name,'group_identifier',group_identifier)
FROM local_file_items WHERE is_accessible=1 AND EXISTS(SELECT 1 FROM local_file_snapshots);
