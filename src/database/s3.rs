use rusqlite::params;

use super::{Database, DatabaseError};
use crate::s3::Object;

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub objects: u64,
    pub prefixes: u64,
    pub bytes: u64,
    pub size_label: String,
    pub completed_at: String,
}

pub fn synchronize(
    database: &Database,
    remote_name: &str,
    objects: &[Object],
) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "UPDATE s3_objects SET is_accessible = 0 WHERE remote_name = ?1",
        [remote_name],
    )?;
    for object in objects {
        transaction.execute(
            "INSERT INTO s3_objects
                (remote_name, object_path, name, size_bytes, modified_at, is_directory, mime_type, checksum, is_accessible, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, CURRENT_TIMESTAMP)
             ON CONFLICT(remote_name, object_path) DO UPDATE SET
                name=excluded.name, size_bytes=excluded.size_bytes,
                modified_at=excluded.modified_at, is_directory=excluded.is_directory,
                mime_type=excluded.mime_type, checksum=excluded.checksum,
                is_accessible=1, last_seen_at=CURRENT_TIMESTAMP",
            params![remote_name, object.path, object.name, object.size_bytes as i64,
                object.modified_at, object.is_directory, object.mime_type, object.checksum],
        )?;
    }
    transaction.execute(
        "INSERT INTO settings(key,value,updated_at) VALUES('s3.last_sync_at',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)
         ON CONFLICT(key) DO UPDATE SET value=CURRENT_TIMESTAMP,updated_at=CURRENT_TIMESTAMP",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn summary(database: &Database) -> Result<Summary, DatabaseError> {
    let connection = database.connect()?;
    let mut summary = connection.query_row(
        "SELECT COALESCE(SUM(CASE WHEN is_directory=0 THEN 1 ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN is_directory=1 THEN 1 ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN is_directory=0 THEN size_bytes ELSE 0 END),0),
                COALESCE((SELECT value FROM settings WHERE key='s3.last_sync_at'),'')
         FROM s3_objects WHERE is_accessible=1",
        [],
        |row| {
            Ok(Summary {
                objects: row.get::<_, i64>(0)? as u64,
                prefixes: row.get::<_, i64>(1)? as u64,
                bytes: row.get::<_, i64>(2)? as u64,
                size_label: String::new(),
                completed_at: row.get(3)?,
            })
        },
    )?;
    summary.size_label = format_bytes(summary.bytes);
    Ok(summary)
}

fn format_bytes(bytes: u64) -> String {
    for (unit, divisor) in [
        ("TB", 1_000_000_000_000_u64),
        ("GB", 1_000_000_000),
        ("MB", 1_000_000),
        ("KB", 1_000),
    ] {
        if bytes >= divisor {
            return format!("{:.1} {unit}", bytes as f64 / divisor as f64);
        }
    }
    format!("{bytes} B")
}
