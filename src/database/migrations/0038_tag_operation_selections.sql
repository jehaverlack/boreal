-- Preserve all-page selections across retries, including when a tag changes its own filter.
CREATE TABLE tag_operation_selections (
    operation_id TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    selection_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
