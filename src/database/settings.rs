use rusqlite::{OptionalExtension, Transaction, params};

use super::{Database, DatabaseError};

#[derive(Debug, Clone)]
pub struct InventorySettings {
    pub google_drive_enabled: bool,
    pub automatic_updates: bool,
    pub refresh_interval_hours: u32,
    pub full_reconciliation_days: u32,
    pub update_when_overdue_at_startup: bool,
    pub directory_sheet_enabled: bool,
    pub directory_sheet_url: String,
    pub github_enabled: bool,
    pub github_login: String,
    pub github_last_sync_at: String,
    pub keeper_enabled: bool,
    pub keeper_command: String,
    pub keeper_last_sync_at: String,
    pub local_files_enabled: bool,
    pub local_file_roots: String,
    pub local_exclude_hidden: bool,
    pub local_exclude_caches: bool,
    pub local_exclude_temporary: bool,
    pub local_exclude_patterns: String,
    pub local_files_last_sync_at: String,
    pub s3_enabled: bool,
    pub s3_remote_name: String,
}

impl Default for InventorySettings {
    fn default() -> Self {
        Self {
            google_drive_enabled: false,
            automatic_updates: false,
            refresh_interval_hours: 24,
            full_reconciliation_days: 7,
            update_when_overdue_at_startup: false,
            directory_sheet_enabled: false,
            directory_sheet_url: String::new(),
            github_enabled: false,
            github_login: String::new(),
            github_last_sync_at: String::new(),
            keeper_enabled: false,
            keeper_command: String::new(),
            keeper_last_sync_at: String::new(),
            local_files_enabled: false,
            local_file_roots: dirs::home_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            local_exclude_hidden: true,
            local_exclude_caches: true,
            local_exclude_temporary: true,
            local_exclude_patterns: String::new(),
            local_files_last_sync_at: String::new(),
            s3_enabled: false,
            s3_remote_name: String::new(),
        }
    }
}

impl InventorySettings {
    pub fn validate(&self) -> Result<(), DatabaseError> {
        if !(1..=720).contains(&self.refresh_interval_hours) {
            return Err("Refresh interval must be between 1 and 720 hours".into());
        }

        if !(1..=365).contains(&self.full_reconciliation_days) {
            return Err("Full reconciliation interval must be between 1 and 365 days".into());
        }

        if self.directory_sheet_enabled && self.directory_sheet_url.trim().is_empty() {
            return Err(
                "A Google Sheets URL is required when linked directory import is enabled".into(),
            );
        }
        if self.local_files_enabled {
            let roots = crate::local_files::parse_roots(&self.local_file_roots);
            crate::local_files::validate_roots(&roots)
                .map_err(|e| -> DatabaseError { e.into() })?;
        }
        if self.s3_enabled {
            crate::s3::validate_remote_name(&self.s3_remote_name)
                .map_err(|error| -> DatabaseError { error })?;
        }

        Ok(())
    }
}

pub fn load(database: &Database) -> Result<InventorySettings, DatabaseError> {
    let connection = database.connect()?;
    let defaults = InventorySettings::default();

    Ok(InventorySettings {
        google_drive_enabled: get_bool(
            &connection,
            "google_drive.enabled",
            defaults.google_drive_enabled,
        )?,
        automatic_updates: get_bool(
            &connection,
            "inventory.automatic_updates",
            defaults.automatic_updates,
        )?,
        refresh_interval_hours: get_u32(
            &connection,
            "inventory.refresh_interval_hours",
            defaults.refresh_interval_hours,
        )?,
        full_reconciliation_days: get_u32(
            &connection,
            "inventory.full_reconciliation_days",
            defaults.full_reconciliation_days,
        )?,
        update_when_overdue_at_startup: get_bool(
            &connection,
            "inventory.update_when_overdue_at_startup",
            defaults.update_when_overdue_at_startup,
        )?,
        directory_sheet_enabled: get_bool(
            &connection,
            "directory.sheet_enabled",
            defaults.directory_sheet_enabled,
        )?,
        directory_sheet_url: get(&connection, "directory.sheet_url")?
            .unwrap_or(defaults.directory_sheet_url),
        github_enabled: get_bool(&connection, "github.enabled", defaults.github_enabled)?,
        github_login: get(&connection, "github.login")?.unwrap_or(defaults.github_login),
        github_last_sync_at: get(&connection, "github.last_sync_at")?
            .unwrap_or(defaults.github_last_sync_at),
        keeper_enabled: get_bool(&connection, "keeper.enabled", defaults.keeper_enabled)?,
        keeper_command: get(&connection, "keeper.command")?.unwrap_or(defaults.keeper_command),
        keeper_last_sync_at: get(&connection, "keeper.last_sync_at")?
            .unwrap_or(defaults.keeper_last_sync_at),
        local_files_enabled: get_bool(
            &connection,
            "local_files.enabled",
            defaults.local_files_enabled,
        )?,
        local_file_roots: get(&connection, "local_files.roots")?
            .unwrap_or(defaults.local_file_roots),
        local_exclude_hidden: get_bool(
            &connection,
            "local_files.exclude_hidden",
            defaults.local_exclude_hidden,
        )?,
        local_exclude_caches: get_bool(
            &connection,
            "local_files.exclude_caches",
            defaults.local_exclude_caches,
        )?,
        local_exclude_temporary: get_bool(
            &connection,
            "local_files.exclude_temporary",
            defaults.local_exclude_temporary,
        )?,
        local_exclude_patterns: get(&connection, "local_files.exclude_patterns")?
            .unwrap_or(defaults.local_exclude_patterns),
        local_files_last_sync_at: get(&connection, "local_files.last_sync_at")?
            .unwrap_or(defaults.local_files_last_sync_at),
        s3_enabled: get_bool(&connection, "s3.enabled", defaults.s3_enabled)?,
        s3_remote_name: get(&connection, "s3.remote_name")?.unwrap_or(defaults.s3_remote_name),
    })
}

pub fn save(database: &Database, settings: &InventorySettings) -> Result<(), DatabaseError> {
    settings.validate()?;

    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;

    set(
        &transaction,
        "google_drive.enabled",
        bool_value(settings.google_drive_enabled),
    )?;
    set(
        &transaction,
        "inventory.automatic_updates",
        bool_value(settings.automatic_updates),
    )?;
    set(
        &transaction,
        "inventory.refresh_interval_hours",
        &settings.refresh_interval_hours.to_string(),
    )?;
    set(
        &transaction,
        "inventory.full_reconciliation_days",
        &settings.full_reconciliation_days.to_string(),
    )?;
    set(
        &transaction,
        "inventory.update_when_overdue_at_startup",
        bool_value(settings.update_when_overdue_at_startup),
    )?;
    set(
        &transaction,
        "directory.sheet_enabled",
        bool_value(settings.directory_sheet_enabled),
    )?;
    set(
        &transaction,
        "directory.sheet_url",
        settings.directory_sheet_url.trim(),
    )?;
    set(
        &transaction,
        "github.enabled",
        bool_value(settings.github_enabled),
    )?;
    set(&transaction, "github.login", settings.github_login.trim())?;
    set(
        &transaction,
        "keeper.enabled",
        bool_value(settings.keeper_enabled),
    )?;
    set(
        &transaction,
        "keeper.command",
        settings.keeper_command.trim(),
    )?;
    for (key, value) in [
        (
            "local_files.enabled",
            bool_value(settings.local_files_enabled),
        ),
        ("local_files.roots", settings.local_file_roots.trim()),
        (
            "local_files.exclude_hidden",
            bool_value(settings.local_exclude_hidden),
        ),
        (
            "local_files.exclude_caches",
            bool_value(settings.local_exclude_caches),
        ),
        (
            "local_files.exclude_temporary",
            bool_value(settings.local_exclude_temporary),
        ),
        (
            "local_files.exclude_patterns",
            settings.local_exclude_patterns.trim(),
        ),
    ] {
        set(&transaction, key, value)?;
    }
    set(&transaction, "s3.enabled", bool_value(settings.s3_enabled))?;
    set(
        &transaction,
        "s3.remote_name",
        settings.s3_remote_name.trim().trim_end_matches(':'),
    )?;
    transaction.execute(
        "INSERT INTO directory_sources (
            name, source_type, source_location, enabled, refresh_on_metadata_update
         ) VALUES ('Linked Google Sheet directory', 'google_sheet', ?1, ?2, ?2)
         ON CONFLICT(name) DO UPDATE SET
            source_location = excluded.source_location,
            enabled = excluded.enabled,
            refresh_on_metadata_update = excluded.refresh_on_metadata_update,
            updated_at = CURRENT_TIMESTAMP",
        params![
            settings.directory_sheet_url.trim(),
            settings.directory_sheet_enabled,
        ],
    )?;

    transaction.commit()?;

    Ok(())
}

/// Preserve Google Drive as enabled for installations that predate modular
/// sources. Fresh installations remain opt-in.
pub fn initialize_google_drive_enabled(
    database: &Database,
    has_google_client: bool,
) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    if get(&connection, "google_drive.enabled")?.is_some() {
        return Ok(());
    }
    let has_drive_data: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM drive_items LIMIT 1)
             OR EXISTS(SELECT 1 FROM remote_accounts LIMIT 1)",
        [],
        |row| row.get(0),
    )?;
    let transaction = connection.transaction()?;
    set(
        &transaction,
        "google_drive.enabled",
        bool_value(has_google_client || has_drive_data),
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn directory_setup_skipped(database: &Database) -> Result<bool, DatabaseError> {
    let connection = database.connect()?;
    get_bool(&connection, "directory.setup_skipped", false)
}

pub fn set_directory_setup_skipped(
    database: &Database,
    skipped: bool,
) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    set(&transaction, "directory.setup_skipped", bool_value(skipped))?;
    transaction.commit()?;
    Ok(())
}

pub fn metadata_setup_skipped(database: &Database) -> Result<bool, DatabaseError> {
    let connection = database.connect()?;
    get_bool(&connection, "metadata.setup_skipped", false)
}

pub fn set_metadata_setup_skipped(database: &Database, skipped: bool) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    set(&transaction, "metadata.setup_skipped", bool_value(skipped))?;
    transaction.commit()?;
    Ok(())
}

pub fn bookmark_reminder_dismissed(database: &Database) -> Result<bool, DatabaseError> {
    let connection = database.connect()?;
    get_bool(&connection, "ui.bookmark_reminder_dismissed", false)
}

pub fn set_bookmark_reminder_dismissed(
    database: &Database,
    dismissed: bool,
) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    set(
        &transaction,
        "ui.bookmark_reminder_dismissed",
        bool_value(dismissed),
    )?;
    transaction.commit()?;
    Ok(())
}

fn get_bool(
    connection: &rusqlite::Connection,
    key: &str,
    default: bool,
) -> Result<bool, DatabaseError> {
    let value = get(connection, key)?;

    match value.as_deref() {
        None => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(value) => Err(format!("Invalid boolean setting {key}: {value}").into()),
    }
}

fn get_u32(
    connection: &rusqlite::Connection,
    key: &str,
    default: u32,
) -> Result<u32, DatabaseError> {
    match get(connection, key)? {
        None => Ok(default),
        Some(value) => value
            .parse::<u32>()
            .map_err(|error| format!("Invalid numeric setting {key}: {error}").into()),
    }
}

fn get(connection: &rusqlite::Connection, key: &str) -> Result<Option<String>, DatabaseError> {
    Ok(connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

fn set(transaction: &Transaction<'_>, key: &str, value: &str) -> Result<(), DatabaseError> {
    transaction.execute(
        "INSERT INTO settings (
            key,
            value,
            updated_at
        ) VALUES (?1, ?2, CURRENT_TIMESTAMP)
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = CURRENT_TIMESTAMP",
        params![key, value,],
    )?;

    Ok(())
}

pub(crate) fn set_in_transaction(
    transaction: &Transaction<'_>,
    key: &str,
    value: &str,
) -> Result<(), DatabaseError> {
    set(transaction, key, value)
}

fn bool_value(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
