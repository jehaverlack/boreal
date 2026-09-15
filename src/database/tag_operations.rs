use super::{Database, DatabaseError};
use rusqlite::{OptionalExtension, params};

/// Freeze the target IDs for a browser operation before its first write. Applying or
/// removing a tag is idempotent, so retrying uses these same IDs even if filters changed.
/// Operation IDs contain a creation timestamp; expired retries are rejected rather than
/// silently resolving a different selection after cache cleanup.
pub fn selection(
    database: &Database,
    operation_id: &str,
    fingerprint: &str,
    resolve: impl FnOnce() -> Result<(Vec<String>, Vec<String>), DatabaseError>,
) -> Result<(Vec<String>, Vec<String>), DatabaseError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let timestamp = operation_id
        .split_once(':')
        .and_then(|(time, _)| time.parse::<u64>().ok())
        .ok_or("Invalid tag operation ID")?;
    if operation_id.len() > 128 || timestamp > now + 300 || now.saturating_sub(timestamp) > 86400 {
        return Err("Tag operation expired; refresh and select items again".into());
    }
    let connection = database.connect()?;
    connection.execute(
        "DELETE FROM tag_operation_selections WHERE created_at < ?1",
        [now.saturating_sub(90000) as i64],
    )?;
    let existing: Option<(String, String)> = connection.query_row(
        "SELECT fingerprint, selection_json FROM tag_operation_selections WHERE operation_id = ?1",
        [operation_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    if let Some((saved_fingerprint, json)) = existing {
        if saved_fingerprint != fingerprint {
            return Err("Tag operation ID reused for another change".into());
        }
        return Ok(serde_json::from_str(&json)?);
    }
    let selected = resolve()?;
    connection.execute(
        "INSERT OR IGNORE INTO tag_operation_selections(operation_id, fingerprint, selection_json, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![operation_id, fingerprint, serde_json::to_string(&selected)?, now as i64],
    )?;
    // A concurrent retry may have inserted its snapshot first.
    let (saved_fingerprint, json): (String, String) = connection.query_row(
        "SELECT fingerprint, selection_json FROM tag_operation_selections WHERE operation_id = ?1",
        [operation_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if saved_fingerprint != fingerprint {
        return Err("Tag operation ID reused for another change".into());
    }
    Ok(serde_json::from_str(&json)?)
}
