use super::{Database, DatabaseError};
use rusqlite::params;

pub struct Record {
    pub item_id: String,
    pub name: String,
    pub path: String,
    pub owner: String,
    pub last_seen: String,
    pub removed_at: String,
    pub removed: bool,
    pub metadata: String,
}

pub struct Version {
    pub recorded_at: String,
    pub metadata: String,
}

/// A flat inventory avoids losing historical children behind moved or missing parents.
pub fn list(
    db: &Database,
    scope: &str,
    search: &str,
    removed_only: bool,
    item: &str,
    offset: usize,
    limit: usize,
) -> Result<(Vec<Record>, usize), DatabaseError> {
    let c = db.connect()?;
    let filter = "FROM drive_items WHERE remote_name=?1 AND (?2='' OR instr(lower(name),lower(?2))>0 OR instr(lower(relative_path),lower(?2))>0 OR instr(lower(COALESCE(owner_email,'')),lower(?2))>0 OR item_id=?2) AND (?3=0 OR is_deleted=1) AND (?4='' OR item_id=?4)";
    let total = c.query_row(
        &format!("SELECT COUNT(*) {filter}"),
        params![scope, search, removed_only, item],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let limit = limit.clamp(1, 200);
    let offset = offset.min(total.saturating_sub(1) / limit * limit);
    let mut s=c.prepare(&format!("SELECT item_id,name,relative_path,COALESCE(owner_email,''),last_seen_at,COALESCE(deleted_at,''),is_deleted,metadata_json {filter} ORDER BY is_deleted DESC,COALESCE(deleted_at,last_seen_at) DESC,item_id LIMIT ?5 OFFSET ?6"))?;
    let rows = s
        .query_map(
            params![
                scope,
                search,
                removed_only,
                item,
                limit.min(200) as i64,
                offset as i64
            ],
            |r| {
                Ok(Record {
                    item_id: r.get(0)?,
                    name: r.get(1)?,
                    path: r.get(2)?,
                    owner: r.get(3)?,
                    last_seen: r.get(4)?,
                    removed_at: r.get(5)?,
                    removed: r.get(6)?,
                    metadata: r.get(7)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok((rows, total))
}

pub fn versions(
    db: &Database,
    scope: &str,
    item: &str,
    offset: usize,
    limit: usize,
) -> Result<(Vec<Version>, usize), DatabaseError> {
    let c = db.connect()?;
    let total = c.query_row(
        "SELECT COUNT(*) FROM drive_item_history WHERE remote_name=?1 AND item_id=?2",
        params![scope, item],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let limit = limit.clamp(1, 200);
    let offset = offset.min(total.saturating_sub(1) / limit * limit);
    let mut s=c.prepare("SELECT recorded_at,record_json FROM drive_item_history WHERE remote_name=?1 AND item_id=?2 ORDER BY id DESC LIMIT ?3 OFFSET ?4")?;
    let rows = s
        .query_map(
            params![scope, item, limit.min(200) as i64, offset as i64],
            |r| {
                Ok(Version {
                    recorded_at: r.get(0)?,
                    metadata: r.get(1)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok((rows, total))
}
