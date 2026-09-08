ALTER TABLE local_file_items ADD COLUMN is_symlink INTEGER NOT NULL DEFAULT 0;
ALTER TABLE local_file_items ADD COLUMN symlink_target TEXT NOT NULL DEFAULT '';

UPDATE tags
SET description = 'Content the user has reviewed and marked as ready for manual deletion from its source location.'
WHERE slug = 'safe-to-delete';

-- Generic review and lifecycle tags apply to repositories too. Migration-only
-- tags remain limited to sources supported by the migration workflow.
INSERT OR IGNORE INTO tag_scopes(tag_id, scope)
SELECT id, 'github-repositories' FROM tags
WHERE slug IN (
    'needs-review', 'permission-review', 'needs-handoff', 'retain',
    'to-delete', 'safe-to-delete'
);

-- Local ownership makes handoff review meaningful, while permission-specific
-- tags remain excluded until filesystem mode/ACL inventory is implemented.
INSERT OR IGNORE INTO tag_scopes(tag_id, scope)
SELECT id, 'local-files' FROM tags WHERE slug = 'needs-handoff';
