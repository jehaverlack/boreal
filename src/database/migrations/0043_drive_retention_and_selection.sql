CREATE TABLE drive_item_history (
    id INTEGER PRIMARY KEY,
    remote_name TEXT NOT NULL,
    item_id TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    record_json TEXT NOT NULL
);
CREATE INDEX drive_history_item_idx ON drive_item_history(remote_name,item_id,id DESC);
CREATE TRIGGER retain_prior_drive_metadata BEFORE UPDATE ON drive_items
WHEN OLD.name IS NOT NEW.name OR
    OLD.relative_path IS NOT NEW.relative_path OR
    OLD.parent_path IS NOT NEW.parent_path OR
    OLD.is_directory IS NOT NEW.is_directory OR
    OLD.mime_type IS NOT NEW.mime_type OR
    OLD.size_bytes IS NOT NEW.size_bytes OR
    OLD.modified_at IS NOT NEW.modified_at OR
    OLD.created_at IS NOT NEW.created_at OR
    OLD.owner_email IS NOT NEW.owner_email OR
    OLD.metadata_json IS NOT NEW.metadata_json OR
    OLD.is_deleted IS NOT NEW.is_deleted OR
    OLD.deleted_at IS NOT NEW.deleted_at
BEGIN
    INSERT INTO drive_item_history(remote_name,item_id,record_json)
    VALUES(OLD.remote_name,OLD.item_id,json_object('remote_name',OLD.remote_name,'item_id',OLD.item_id,'name',OLD.name,'relative_path',OLD.relative_path,'parent_path',OLD.parent_path,'is_directory',OLD.is_directory,'mime_type',OLD.mime_type,'size_bytes',OLD.size_bytes,'cumulative_size_bytes',OLD.cumulative_size_bytes,'modified_at',OLD.modified_at,'created_at',OLD.created_at,'owner_email',OLD.owner_email,'metadata_json',OLD.metadata_json,'first_seen_at',OLD.first_seen_at,'last_seen_at',OLD.last_seen_at,'last_seen_scan_id',OLD.last_seen_scan_id,'is_deleted',OLD.is_deleted,'deleted_at',OLD.deleted_at,'permissions',json((SELECT COALESCE(json_group_array(json(raw_json)),'[]') FROM drive_permissions WHERE remote_name=OLD.remote_name AND item_id=OLD.item_id))));
END;
CREATE TRIGGER retain_drive_items BEFORE DELETE ON drive_items
BEGIN
    SELECT RAISE(ABORT,'Drive inventory records must be retained; mark unavailable instead');
END;
CREATE TRIGGER retain_drive_history BEFORE DELETE ON drive_item_history
BEGIN
    SELECT RAISE(ABORT,'Prior Drive metadata must be retained');
END;
ALTER TABLE migration_jobs ADD COLUMN selection_url TEXT NOT NULL DEFAULT '';
ALTER TABLE migration_jobs ADD COLUMN revised_from INTEGER;
