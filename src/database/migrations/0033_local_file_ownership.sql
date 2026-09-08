ALTER TABLE principals ADD COLUMN username TEXT COLLATE NOCASE;
CREATE UNIQUE INDEX principals_username_idx
    ON principals (username COLLATE NOCASE)
    WHERE username IS NOT NULL AND trim(username) <> '';

ALTER TABLE local_file_items ADD COLUMN owner_username TEXT NOT NULL DEFAULT '';
ALTER TABLE local_file_items ADD COLUMN owner_identifier TEXT NOT NULL DEFAULT '';
ALTER TABLE local_file_items ADD COLUMN group_name TEXT NOT NULL DEFAULT '';
ALTER TABLE local_file_items ADD COLUMN group_identifier TEXT NOT NULL DEFAULT '';

CREATE INDEX local_file_items_owner_username_idx
    ON local_file_items (owner_username COLLATE NOCASE);
