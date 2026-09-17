ALTER TABLE local_file_items ADD COLUMN parent_path TEXT NOT NULL DEFAULT '';
ALTER TABLE local_file_items ADD COLUMN duplicate_copies INTEGER NOT NULL DEFAULT 0;
UPDATE local_file_items SET parent_path=rtrim(rtrim(relative_path,replace(relative_path,'/','')),'/');
CREATE INDEX local_file_children_idx ON local_file_items(root_path,parent_path,is_directory DESC,name COLLATE NOCASE,id) WHERE is_accessible=1;
CREATE INDEX local_file_active_checksum_idx ON local_file_items(checksum_sha256) WHERE is_accessible=1 AND checksum_sha256<>'';
CREATE INDEX principals_username_lower_idx ON principals(lower(username));
CREATE TABLE local_file_summary (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    files INTEGER NOT NULL, folders INTEGER NOT NULL, bytes INTEGER NOT NULL,
    duplicate_groups INTEGER NOT NULL, duplicate_bytes INTEGER NOT NULL
);
WITH counts AS (
    SELECT checksum_sha256, COUNT(*) AS copies FROM local_file_items
    WHERE is_accessible=1 AND checksum_sha256<>'' GROUP BY checksum_sha256
)
UPDATE local_file_items SET duplicate_copies=COALESCE((
    SELECT copies FROM counts WHERE counts.checksum_sha256=local_file_items.checksum_sha256
),0) WHERE is_accessible=1;
INSERT OR REPLACE INTO local_file_summary
(singleton,files,folders,bytes,duplicate_groups,duplicate_bytes)
SELECT 1,
    COALESCE(SUM(is_directory=0),0), COALESCE(SUM(is_directory=1),0),
    COALESCE(SUM(CASE WHEN is_directory=0 THEN size_bytes ELSE 0 END),0),
    (SELECT COUNT(*) FROM (SELECT checksum_sha256 FROM local_file_items WHERE is_accessible=1 AND duplicate_copies>1 GROUP BY checksum_sha256)),
    (SELECT COALESCE(SUM((copies-1)*size_bytes),0) FROM (SELECT size_bytes,COUNT(*) copies FROM local_file_items WHERE is_accessible=1 AND checksum_sha256<>'' GROUP BY checksum_sha256,size_bytes HAVING COUNT(*)>1))
FROM local_file_items WHERE is_accessible=1;
