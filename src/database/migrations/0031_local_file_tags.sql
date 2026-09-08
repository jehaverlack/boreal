ALTER TABLE tag_scopes RENAME TO tag_scopes_old;
CREATE TABLE tag_scopes (
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    scope TEXT NOT NULL CHECK (scope IN (
        'directory', 'my-drive', 'shared-drives', 'shared-with-me',
        'github-repositories', 'keeper-shared-folders', 'local-files'
    )),
    PRIMARY KEY (tag_id, scope)
);
INSERT INTO tag_scopes(tag_id,scope) SELECT tag_id,scope FROM tag_scopes_old;
DROP TABLE tag_scopes_old;
CREATE INDEX idx_tag_scopes_scope ON tag_scopes(scope,tag_id);

CREATE TABLE local_file_tags (
    local_file_id INTEGER NOT NULL REFERENCES local_file_items(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(local_file_id,tag_id)
);
CREATE INDEX local_file_tags_tag ON local_file_tags(tag_id,local_file_id);

INSERT OR IGNORE INTO tag_scopes(tag_id,scope)
SELECT id,'local-files' FROM tags
WHERE slug IN ('needs-review','retain','to-delete','safe-to-delete');
