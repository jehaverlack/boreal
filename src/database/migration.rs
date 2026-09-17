use rusqlite::{OptionalExtension, params};

use super::{Database, DatabaseError};

#[derive(Debug, Clone)]
pub struct MigrationJob {
    pub id: i64,
    pub source_kind: String,
    pub source_scope: String,
    pub operation_kind: String,
    pub status: String,
    pub phase: String,
    pub destination_url: String,
    pub destination_kind: String,
    pub destination_path: String,
    pub destination_drive_name: String,
    pub destination_folder_name: String,
    pub destination_drive_id: String,
    pub destination_folder_id: String,
    pub files_total: u64,
    pub folders_total: u64,
    pub bytes_total: u64,
    pub files_copied: u64,
    pub bytes_copied: u64,
    pub exceptions_count: u64,
    pub created_at: String,
    pub started_at: String,
    pub completed_at: String,
    pub copy_completed_at: String,
    pub archived_at: String,
    pub resume_count: u64,
    pub error_message: String,
    pub sources: Vec<MigrationSource>,
}

#[derive(Debug, Clone)]
pub struct MigrationSource {
    pub item_id: String,
    pub name: String,
    pub relative_path: String,
    pub is_directory: bool,
    pub files_total: u64,
    pub folders_total: u64,
    pub bytes_total: u64,
    pub status: String,
    pub error_message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveMigrationSummary {
    pub count: usize,
    pub files_total: u64,
    pub files_copied: u64,
    pub bytes_total: u64,
    pub bytes_copied: u64,
}

pub fn active_count(database: &Database) -> Result<usize, DatabaseError> {
    let count = database.connect()?.query_row(
        "SELECT COUNT(*) FROM migration_jobs WHERE status IN ('preflight', 'running')",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(count as usize)
}

pub fn active_summary(database: &Database) -> Result<ActiveMigrationSummary, DatabaseError> {
    let summary = database.connect()?.query_row(
        "SELECT COUNT(*), COALESCE(SUM(files_total), 0), COALESCE(SUM(files_copied), 0),
                COALESCE(SUM(bytes_total), 0), COALESCE(SUM(bytes_copied), 0)
         FROM migration_jobs WHERE status IN ('preflight', 'running')",
        [],
        |row| {
            Ok(ActiveMigrationSummary {
                count: row.get::<_, i64>(0)? as usize,
                files_total: row.get::<_, i64>(1)? as u64,
                files_copied: row.get::<_, i64>(2)? as u64,
                bytes_total: row.get::<_, i64>(3)? as u64,
                bytes_copied: row.get::<_, i64>(4)? as u64,
            })
        },
    )?;
    Ok(summary)
}

pub fn create(
    database: &Database,
    source_scope: &str,
    source_kind: &str,
    item_ids: &[String],
    operation_kind: &str,
) -> Result<i64, DatabaseError> {
    if !matches!(source_kind, "my-drive" | "shared-with-me" | "shared-drive")
        || !matches!(operation_kind, "drive-copy" | "local-download")
        || item_ids.is_empty()
    {
        return Err("Select at least one Drive item".into());
    }
    if source_kind == "shared-drive" {
        let drive_id = source_scope
            .strip_prefix(super::inventory::SHARED_DRIVE_SCOPE_PREFIX)
            .filter(|id| {
                !id.is_empty()
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
            .ok_or("Invalid Shared Drive source")?;
        let drive = super::inventory::get_shared_drive(database, drive_id)?
            .ok_or("Shared Drive is not indexed")?;
        if !drive.is_accessible || drive.inventory_scope != source_scope {
            return Err("Shared Drive source is unavailable".into());
        }
    }
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO migration_jobs (source_scope, source_kind, operation_kind)
         VALUES (?1, ?2, ?3)",
        params![source_scope, source_kind, operation_kind],
    )?;
    let migration_id = transaction.last_insert_rowid();
    for item_id in item_ids {
        let source = transaction
            .query_row(
                "SELECT item_id, name, relative_path, is_directory
                 FROM drive_items
                 WHERE remote_name = ?1 AND item_id = ?2 AND is_deleted = 0",
                params![source_scope, item_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| format!("Unknown or deleted migration source: {item_id}"))?;
        let pattern = format!("{}/%", source.2);
        let (files, folders, bytes): (i64, i64, i64) = transaction.query_row(
            "SELECT
                SUM(CASE WHEN is_directory = 0 THEN 1 ELSE 0 END),
                SUM(CASE WHEN is_directory = 1 THEN 1 ELSE 0 END),
                COALESCE(SUM(CASE WHEN is_directory = 0 THEN COALESCE(size_bytes, 0) ELSE 0 END), 0)
             FROM drive_items
             WHERE remote_name = ?1 AND is_deleted = 0
               AND (item_id = ?2 OR (?3 = 1 AND relative_path LIKE ?4))",
            params![source_scope, item_id, source.3, pattern],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        transaction.execute(
            "INSERT INTO migration_sources
             (migration_id, item_id, name, relative_path, is_directory, files_total, folders_total, bytes_total)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![migration_id, source.0, source.1, source.2, source.3, files, folders, bytes],
        )?;
    }
    transaction.execute(
        "UPDATE migration_jobs SET
            files_total = (SELECT COALESCE(SUM(files_total), 0) FROM migration_sources WHERE migration_id = ?1),
            folders_total = (SELECT COALESCE(SUM(folders_total), 0) FROM migration_sources WHERE migration_id = ?1),
            bytes_total = (SELECT COALESCE(SUM(bytes_total), 0) FROM migration_sources WHERE migration_id = ?1)
         WHERE id = ?1",
        [migration_id],
    )?;
    transaction.commit()?;
    Ok(migration_id)
}

pub fn create_shared_drive_download(
    database: &Database,
    drive_id: &str,
) -> Result<i64, DatabaseError> {
    let drive = crate::database::inventory::get_shared_drive(database, drive_id)?
        .ok_or_else(|| format!("Unknown Shared Drive: {drive_id}"))?;
    if !drive.is_accessible {
        return Err("The Shared Drive is not accessible".into());
    }
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO migration_jobs
         (source_scope, source_kind, operation_kind, files_total, folders_total, bytes_total)
         VALUES (?1, 'shared-drive', 'local-download', ?2, ?3, ?4)",
        params![
            drive.inventory_scope,
            drive.files_scanned as i64,
            drive.folders_scanned as i64,
            drive.bytes_discovered as i64
        ],
    )?;
    let id = transaction.last_insert_rowid();
    transaction.execute(
        "INSERT INTO migration_sources
         (migration_id, item_id, name, relative_path, is_directory,
          files_total, folders_total, bytes_total)
         VALUES (?1, ?2, ?3, '', 1, ?4, ?5, ?6)",
        params![
            id,
            drive.drive_id,
            drive.name,
            drive.files_scanned as i64,
            drive.folders_scanned as i64,
            drive.bytes_discovered as i64
        ],
    )?;
    transaction.commit()?;
    Ok(id)
}

pub fn list(
    database: &Database,
    search: &str,
    include_archived: bool,
    sort: &str,
    descending: bool,
) -> Result<Vec<MigrationJob>, DatabaseError> {
    let connection = database.connect()?;
    let order_column = match sort {
        "id" => "mj.id",
        "source" => "mj.source_kind",
        "status" => "mj.status",
        "destination" => {
            "CASE WHEN mj.destination_kind = 'local' THEN mj.destination_path ELSE mj.destination_drive_name END COLLATE NOCASE"
        }
        "files" => "mj.files_total",
        "folders" => "mj.folders_total",
        "capacity" => "mj.bytes_total",
        "completed" => "mj.completed_at",
        _ => "mj.created_at",
    };
    let direction = if descending { "DESC" } else { "ASC" };
    let sql = format!(
        "SELECT mj.id, mj.source_kind, mj.operation_kind, mj.status, mj.phase,
                mj.destination_url, mj.destination_kind, mj.destination_path, mj.destination_drive_name,
                destination_folder_name, files_total, folders_total, bytes_total,
                files_copied, bytes_copied, exceptions_count, created_at,
                COALESCE(started_at, ''), COALESCE(completed_at, ''), error_message,
                COALESCE(archived_at, ''), destination_drive_id, destination_folder_id,
                COALESCE(copy_completed_at, ''), resume_count, source_scope
         FROM migration_jobs mj
         WHERE (?1 OR mj.archived_at IS NULL)
           AND (?2 = '' OR CAST(mj.id AS TEXT) LIKE ?3 OR mj.source_kind LIKE ?3
                OR mj.operation_kind LIKE ?3 OR mj.status LIKE ?3 OR mj.phase LIKE ?3
                OR mj.destination_drive_name LIKE ?3 OR mj.destination_folder_name LIKE ?3
                OR mj.destination_path LIKE ?3
                OR mj.created_at LIKE ?3 OR COALESCE(mj.completed_at, '') LIKE ?3
                OR EXISTS (SELECT 1 FROM migration_sources ms
                           WHERE ms.migration_id = mj.id
                             AND (ms.name LIKE ?3 OR ms.relative_path LIKE ?3)))
         ORDER BY {order_column} {direction}, mj.id {direction}"
    );
    let mut statement = connection.prepare(&sql)?;
    let pattern = format!("%{}%", search.trim());
    let mut jobs = statement
        .query_map(
            params![include_archived, search.trim(), pattern],
            job_from_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for job in &mut jobs {
        job.sources = load_sources(&connection, job.id)?;
    }
    Ok(jobs)
}

pub fn get(database: &Database, id: i64) -> Result<Option<MigrationJob>, DatabaseError> {
    let connection = database.connect()?;
    let mut job = connection
        .query_row(
            "SELECT id, source_kind, operation_kind, status, phase,
                    destination_url, destination_kind, destination_path, destination_drive_name,
                    destination_folder_name, files_total, folders_total, bytes_total,
                    files_copied, bytes_copied, exceptions_count, created_at,
                    COALESCE(started_at, ''), COALESCE(completed_at, ''), error_message
                    , COALESCE(archived_at, ''), destination_drive_id, destination_folder_id,
                    COALESCE(copy_completed_at, ''), resume_count, source_scope
             FROM migration_jobs WHERE id = ?1",
            [id],
            job_from_row,
        )
        .optional()?;
    if let Some(job) = &mut job {
        job.sources = load_sources(&connection, id)?;
    }
    Ok(job)
}

fn load_sources(
    connection: &rusqlite::Connection,
    id: i64,
) -> Result<Vec<MigrationSource>, DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT item_id, name, relative_path, is_directory, files_total, folders_total, bytes_total,
                status, error_message
         FROM migration_sources WHERE migration_id = ?1 ORDER BY name COLLATE NOCASE",
    )?;
    statement
        .query_map([id], |row| {
            Ok(MigrationSource {
                item_id: row.get(0)?,
                name: row.get(1)?,
                relative_path: row.get(2)?,
                is_directory: row.get(3)?,
                files_total: row.get::<_, i64>(4)? as u64,
                folders_total: row.get::<_, i64>(5)? as u64,
                bytes_total: row.get::<_, i64>(6)? as u64,
                status: row.get(7)?,
                error_message: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub fn cancel(database: &Database, id: i64) -> Result<(), DatabaseError> {
    let connection = database.connect()?;
    let changed = connection.execute(
        "DELETE FROM migration_jobs
         WHERE id = ?1 AND started_at IS NULL AND status IN ('draft', 'ready')",
        [id],
    )?;
    if changed == 0 {
        return Err("Only migrations that have not started can be canceled".into());
    }
    Ok(())
}

pub fn begin_copy(database: &Database, id: i64) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let changed = transaction.execute(
        "UPDATE migration_jobs
         SET status = 'preflight', phase = 'Validating destination write access',
             updated_at = CURRENT_TIMESTAMP, error_message = '', completed_at = NULL,
             copy_completed_at = NULL, files_copied = 0, bytes_copied = 0,
             resume_count = resume_count + CASE WHEN started_at IS NULL THEN 0 ELSE 1 END
         WHERE id = ?1 AND status IN ('ready', 'interrupted', 'error', 'copied')
           AND archived_at IS NULL",
        [id],
    )?;
    if changed == 0 {
        return Err("This migration cannot be started or resumed".into());
    }
    transaction.execute(
        "UPDATE migration_sources SET status = 'pending', started_at = NULL,
             completed_at = NULL, error_message = '' WHERE migration_id = ?1",
        [id],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn confirm_copy_started(database: &Database, id: i64) -> Result<(), DatabaseError> {
    let changed = database.connect()?.execute(
        "UPDATE migration_jobs SET status = 'running', phase = 'Copying selected items',
             started_at = COALESCE(started_at, CURRENT_TIMESTAMP), updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1 AND status = 'preflight'",
        [id],
    )?;
    if changed == 0 {
        return Err("Migration preflight is no longer active".into());
    }
    Ok(())
}

pub fn fail_preflight(database: &Database, id: i64, error: &str) -> Result<(), DatabaseError> {
    database.connect()?.execute(
        "UPDATE migration_jobs SET status = CASE WHEN started_at IS NULL THEN 'ready' ELSE 'error' END,
             phase = 'Preflight requires attention',
             error_message = ?2, updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1 AND status = 'preflight'",
        params![id, error],
    )?;
    Ok(())
}

pub fn start_source(database: &Database, id: i64, item_id: &str) -> Result<(), DatabaseError> {
    database.connect()?.execute(
        "UPDATE migration_sources SET status = 'running', started_at = CURRENT_TIMESTAMP,
             error_message = '' WHERE migration_id = ?1 AND item_id = ?2",
        params![id, item_id],
    )?;
    Ok(())
}

pub fn complete_source(database: &Database, id: i64, item_id: &str) -> Result<(), DatabaseError> {
    let connection = database.connect()?;
    connection.execute(
        "UPDATE migration_sources SET status = 'completed', completed_at = CURRENT_TIMESTAMP
         WHERE migration_id = ?1 AND item_id = ?2",
        params![id, item_id],
    )?;
    connection.execute(
        "UPDATE migration_jobs SET
             files_copied = (SELECT COALESCE(SUM(files_total), 0) FROM migration_sources
                             WHERE migration_id = ?1 AND status = 'completed'),
             bytes_copied = (SELECT COALESCE(SUM(bytes_total), 0) FROM migration_sources
                             WHERE migration_id = ?1 AND status = 'completed'),
             updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
        [id],
    )?;
    Ok(())
}

pub fn fail_copy(
    database: &Database,
    id: i64,
    item_id: Option<&str>,
    error: &str,
) -> Result<(), DatabaseError> {
    let connection = database.connect()?;
    if let Some(item_id) = item_id {
        connection.execute(
            "UPDATE migration_sources SET status = 'failed', completed_at = CURRENT_TIMESTAMP,
                 error_message = ?3 WHERE migration_id = ?1 AND item_id = ?2",
            params![id, item_id, error],
        )?;
    }
    connection.execute(
        "UPDATE migration_jobs SET status = 'error', phase = 'Copy failed', error_message = ?2,
             exceptions_count = exceptions_count + 1,
             updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
        params![id, error],
    )?;
    Ok(())
}

pub fn complete_copy(database: &Database, id: i64) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let changed = transaction.execute(
        "UPDATE migration_jobs SET status = 'copied',
             phase = CASE WHEN operation_kind = 'local-download'
                          THEN 'Download complete' ELSE 'Copy complete; verification pending' END,
             files_copied = files_total, bytes_copied = bytes_total,
             copy_completed_at = CURRENT_TIMESTAMP, completed_at = CURRENT_TIMESTAMP,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1 AND status = 'running'",
        [id],
    )?;
    if changed == 0 {
        return Err("Only a running migration can complete its copy".into());
    }
    transaction.execute(
        "INSERT OR IGNORE INTO drive_item_tags (remote_name, item_id, tag_id)
         SELECT descendant.remote_name, descendant.item_id, tag.id
         FROM migration_jobs job
         JOIN migration_sources source ON source.migration_id = job.id
         JOIN drive_items selected
           ON selected.remote_name = job.source_scope
          AND selected.item_id = source.item_id
         JOIN drive_items descendant
           ON descendant.remote_name = selected.remote_name
          AND descendant.is_deleted = 0
          AND (
               descendant.item_id = selected.item_id
               OR (selected.is_directory = 1 AND
                   substr(descendant.relative_path, 1, length(selected.relative_path) + 1)
                       = selected.relative_path || '/')
          )
         JOIN tags tag ON tag.slug = 'migrated'
         JOIN tag_scopes scope
           ON scope.tag_id = tag.id
          AND scope.scope = CASE job.source_kind
              WHEN 'my-drive' THEN 'my-drive'
              WHEN 'shared-with-me' THEN 'shared-with-me'
              WHEN 'shared-drive' THEN 'shared-drives'
          END
         WHERE job.id = ?1 AND job.operation_kind = 'drive-copy'
           AND source.status = 'completed'",
        [id],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn archive(database: &Database, id: i64) -> Result<(), DatabaseError> {
    let connection = database.connect()?;
    let changed = connection.execute(
        "UPDATE migration_jobs SET archived_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1 AND status NOT IN ('preflight', 'running') AND archived_at IS NULL",
        [id],
    )?;
    if changed == 0 {
        return Err("Only inactive, unarchived migrations can be archived".into());
    }
    Ok(())
}

pub fn set_destination(
    database: &Database,
    id: i64,
    url: &str,
    drive_id: &str,
    drive_name: &str,
    folder_id: &str,
    folder_name: &str,
) -> Result<(), DatabaseError> {
    let connection = database.connect()?;
    let changed = connection.execute(
        "UPDATE migration_jobs SET destination_kind = 'google-drive', destination_path = '',
            operation_kind = 'drive-copy', destination_url = ?2, destination_drive_id = ?3,
            destination_drive_name = ?4, destination_folder_id = ?5,
            destination_folder_name = ?6, status = 'ready', phase = 'Ready for copy authorization',
            updated_at = CURRENT_TIMESTAMP, error_message = ''
         WHERE id = ?1 AND started_at IS NULL AND status IN ('draft', 'ready')",
        params![id, url, drive_id, drive_name, folder_id, folder_name],
    )?;
    if changed == 0 {
        return Err(format!("Unknown migration: {id}").into());
    }
    Ok(())
}

pub fn set_local_destination(
    database: &Database,
    id: i64,
    destination_path: &str,
) -> Result<(), DatabaseError> {
    let changed = database.connect()?.execute(
        "UPDATE migration_jobs SET destination_kind = 'local', operation_kind = 'local-download',
             destination_path = ?2, destination_url = '', destination_drive_id = '',
             destination_drive_name = 'Local folder', destination_folder_id = '',
             destination_folder_name = ?2, status = 'ready', phase = 'Ready to download',
             updated_at = CURRENT_TIMESTAMP, error_message = ''
         WHERE id = ?1 AND source_kind <> 'local-files' AND started_at IS NULL AND status IN ('draft', 'ready')",
        params![id, destination_path],
    )?;
    if changed == 0 {
        return Err(format!("Unknown or already-started migration: {id}").into());
    }
    Ok(())
}

pub fn recover_interrupted(database: &Database) -> Result<usize, DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let changed = transaction.execute(
        "UPDATE migration_jobs SET status = 'interrupted',
             phase = 'Interrupted; safe to resume', updated_at = CURRENT_TIMESTAMP,
             error_message = 'BOREAL stopped before this transfer completed'
         WHERE status IN ('preflight', 'running')",
        [],
    )?;
    transaction.execute(
        "UPDATE migration_sources SET status = 'pending', started_at = NULL,
             completed_at = NULL WHERE status = 'running'",
        [],
    )?;
    transaction.commit()?;
    Ok(changed)
}

pub fn resolve_destination(
    database: &Database,
    folder_id: &str,
) -> Result<Option<(String, String, String)>, DatabaseError> {
    let connection = database.connect()?;
    connection
        .query_row(
            "SELECT sd.drive_id, sd.name, sd.name FROM shared_drives sd WHERE sd.drive_id = ?1
         UNION ALL
         SELECT sd.drive_id, sd.name, di.name FROM drive_items di
         JOIN shared_drives sd ON sd.inventory_scope = di.remote_name
         WHERE di.item_id = ?1 AND di.is_directory = 1 AND di.is_deleted = 0 LIMIT 1",
            [folder_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(Into::into)
}

fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MigrationJob> {
    Ok(MigrationJob {
        id: row.get(0)?,
        source_kind: row.get(1)?,
        source_scope: row.get(25)?,
        operation_kind: row.get(2)?,
        status: row.get(3)?,
        phase: row.get(4)?,
        destination_url: row.get(5)?,
        destination_kind: row.get(6)?,
        destination_path: row.get(7)?,
        destination_drive_name: row.get(8)?,
        destination_folder_name: row.get(9)?,
        files_total: row.get::<_, i64>(10)? as u64,
        folders_total: row.get::<_, i64>(11)? as u64,
        bytes_total: row.get::<_, i64>(12)? as u64,
        files_copied: row.get::<_, i64>(13)? as u64,
        bytes_copied: row.get::<_, i64>(14)? as u64,
        exceptions_count: row.get::<_, i64>(15)? as u64,
        created_at: row.get(16)?,
        started_at: row.get(17)?,
        completed_at: row.get(18)?,
        error_message: row.get(19)?,
        archived_at: row.get(20)?,
        destination_drive_id: row.get(21)?,
        destination_folder_id: row.get(22)?,
        copy_completed_at: row.get(23)?,
        resume_count: row.get::<_, i64>(24)? as u64,
        sources: Vec::new(),
    })
}
