pub mod directory;
pub mod github;
pub mod inventory;
pub mod keeper;
pub mod local_files;
pub mod migration;
mod migrations;
pub mod s3;
pub mod settings;

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::Connection;
use rusqlite::params;

use crate::bootstrap::Runtime;

pub type DatabaseError = Box<dyn Error + Send + Sync>;

const DATABASE_FILE_NAME: &str = "boreal.sqlite";

#[derive(Debug, Clone)]
pub struct Database {
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MetadataTimingEstimate {
    pub average_seconds: u64,
    pub sample_count: u64,
}

impl Database {
    /// Initialize BOREAL's SQLite database and apply all pending migrations.
    pub fn initialize(runtime: &Runtime) -> Result<Self, DatabaseError> {
        let path = path(runtime)?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let database = Self { path };

        let mut connection = database.connect()?;

        migrations::apply(&mut connection)?;

        Ok(database)
    }

    /// Open a configured connection to the database.
    ///
    /// Connections are intentionally short-lived so future background jobs can
    /// perform database work without sharing a non-Sync SQLite connection.
    pub fn connect(&self) -> Result<Connection, DatabaseError> {
        let connection = Connection::open(&self.path)?;

        configure_connection(&connection)?;

        Ok(connection)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn start_scan_run(&self, scan_type: &str) -> Result<i64, DatabaseError> {
        let connection = self.connect()?;

        connection.execute(
            "INSERT INTO scan_runs (
                scan_type,
                status
            ) VALUES (?1, 'running')",
            [scan_type],
        )?;

        Ok(connection.last_insert_rowid())
    }

    pub fn fail_scan_run(&self, id: i64, error: &str) -> Result<(), DatabaseError> {
        let connection = self.connect()?;

        connection.execute(
            "UPDATE scan_runs
             SET status = 'error',
                 completed_at = CURRENT_TIMESTAMP,
                 error_message = ?2
             WHERE id = ?1
               AND status = 'running'",
            params![id, error,],
        )?;

        Ok(())
    }

    pub fn complete_scan_run(
        &self,
        id: i64,
        summary: &inventory::InventorySummary,
    ) -> Result<(), DatabaseError> {
        let connection = self.connect()?;
        connection.execute(
            "UPDATE scan_runs SET status = 'complete', completed_at = CURRENT_TIMESTAMP,
                    error_message = NULL, files_scanned = ?2, folders_scanned = ?3,
                    permissions_scanned = ?4, bytes_discovered = ?5, deleted_items = ?6
             WHERE id = ?1",
            params![
                id,
                summary.files_scanned as i64,
                summary.folders_scanned as i64,
                summary.permissions_scanned as i64,
                summary.bytes_discovered as i64,
                summary.deleted_items as i64
            ],
        )?;
        Ok(())
    }

    pub fn record_metadata_timing(
        &self,
        source: &str,
        duration_seconds: u64,
    ) -> Result<(), DatabaseError> {
        let connection = self.connect()?;
        connection.execute(
            "INSERT INTO metadata_timing_history(source, duration_seconds)
             VALUES (?1, ?2)",
            params![source, duration_seconds.max(1) as i64],
        )?;
        Ok(())
    }

    pub fn metadata_timing_estimate(
        &self,
        source: &str,
    ) -> Result<Option<MetadataTimingEstimate>, DatabaseError> {
        let connection = self.connect()?;
        let (average, count) = connection.query_row(
            "SELECT COALESCE(AVG(duration_seconds), 0), COUNT(*) FROM (
                 SELECT duration_seconds FROM metadata_timing_history
                 WHERE source = ?1 ORDER BY id DESC LIMIT 5
             )",
            [source],
            |row| Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        Ok((count > 0).then_some(MetadataTimingEstimate {
            average_seconds: average.round() as u64,
            sample_count: count as u64,
        }))
    }
}

fn path(runtime: &Runtime) -> Result<PathBuf, DatabaseError> {
    let sqlite_dir = runtime
        .directories
        .get("SQLITE")
        .ok_or("BOREAL SQLITE directory is not configured")?;

    Ok(sqlite_dir.join(DATABASE_FILE_NAME))
}

fn configure_connection(connection: &Connection) -> Result<(), DatabaseError> {
    connection.busy_timeout(Duration::from_secs(5))?;

    connection.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        ",
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::rclone::inventory::DriveItem;

    use super::*;

    fn runtime(root: &Path) -> Runtime {
        let mut directories = BTreeMap::new();

        directories.insert("SQLITE".to_string(), root.join("data/sqlite"));

        Runtime {
            boreal_home: root.to_path_buf(),
            boreal: serde_json::json!({}),
            directories,
        }
    }

    fn temporary_directory() -> PathBuf {
        let unique = format!(
            "boreal-database-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH,)
                .expect("system clock should be valid",)
                .as_nanos(),
        );

        std::env::temp_dir().join(unique)
    }

    #[test]
    fn initializes_and_reopens_database() {
        let root = temporary_directory();
        let runtime = runtime(&root);

        let first = Database::initialize(&runtime).expect("database should initialize");

        assert!(first.path().is_file(),);

        let second = Database::initialize(&runtime).expect("database should reopen");

        let connection = second.connect().expect("database should connect");

        let migration_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("migration count should be readable");

        assert_eq!(migration_count, 35,);

        let safe_to_delete_scope_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tag_scopes s
                 JOIN tags t ON t.id = s.tag_id
                 WHERE t.slug = 'safe-to-delete'",
                [],
                |row| row.get(0),
            )
            .expect("migrated tag scopes should be readable");
        assert_eq!(safe_to_delete_scope_count, 5);

        for (slug, expected_scope_count) in [
            ("data-loss-risk", 1_i64),
            ("access-review", 1_i64),
            ("needs-review", 6_i64),
            ("permission-review", 5_i64),
            ("needs-handoff", 6_i64),
            ("retain", 6_i64),
            ("to-delete", 5_i64),
            ("to-migrate", 2_i64),
            ("migrated", 2_i64),
            ("remove-my-permissions", 4_i64),
        ] {
            let scope_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM tag_scopes s
                     JOIN tags t ON t.id = s.tag_id
                     WHERE t.slug = ?1",
                    [slug],
                    |row| row.get(0),
                )
                .expect("built-in tag scopes should be readable");
            assert_eq!(scope_count, expected_scope_count, "scope count for {slug}");
        }

        let safe_to_delete_tag: (String, String, String) = connection
            .query_row(
                "SELECT name, description, color FROM tags WHERE slug = 'safe-to-delete'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("safe-to-delete tag should exist");
        assert_eq!(safe_to_delete_tag.0, "Safe to Delete");
        assert_eq!(
            safe_to_delete_tag.1,
            "Content the user has reviewed and marked as ready for manual deletion from its source location."
        );
        assert_eq!(safe_to_delete_tag.2, "#198754");

        let keep_tag: (String, String) = connection
            .query_row(
                "SELECT name, color FROM tags WHERE slug = 'retain'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("Keep tag should exist");
        assert_eq!(keep_tag.0, "Keep");
        assert_eq!(keep_tag.1, "#00d149");

        for slug in [
            "data-loss-risk",
            "access-review",
            "needs-review",
            "permission-review",
            "needs-handoff",
            "retain",
            "to-migrate",
            "migrated",
            "remove-my-permissions",
            "to-delete",
            "safe-to-delete",
        ] {
            let description: String = connection
                .query_row(
                    "SELECT description FROM tags WHERE slug = ?1",
                    [slug],
                    |row| row.get(0),
                )
                .expect("built-in tag description should be readable");
            assert!(!description.is_empty(), "description for {slug}");
        }

        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn owner_candidates_include_custom_person_types_and_email_aliases() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        directory::import_csv(
            &database,
            "people.csv",
            b"email,username,name,type\nresearcher@example.edu,researcher,Research User,Affiliate Researcher\ngroup@example.edu,research-group,Research Group,Google Group\n",
        )
        .expect("directory should import");
        let connection = database.connect().expect("database should connect");
        connection
            .execute(
                "INSERT INTO principal_emails (principal_id, email)
                 SELECT id, 'alias@example.edu' FROM principals WHERE username = 'researcher'",
                [],
            )
            .expect("email alias should insert");
        drop(connection);

        let choices =
            directory::list_person_choices(&database).expect("owner candidates should be readable");
        assert_eq!(choices.len(), 1, "groups should not be owner candidates");
        assert_eq!(choices[0].username, "researcher");
        assert!(choices[0].search_text.contains("alias@example.edu"));

        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn creates_foundation_tables() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        let connection = database.connect().expect("database should connect");

        for table in [
            "schema_migrations",
            "settings",
            "scan_runs",
            "drive_items",
            "drive_permissions",
            "tags",
            "drive_item_tags",
            "organizations",
            "principals",
            "principal_emails",
            "organization_memberships",
            "principal_memberships",
            "directory_sources",
            "directory_import_runs",
            "remote_accounts",
            "metadata_timing_history",
            "s3_objects",
            "shared_drives",
            "shared_drive_tags",
            "github_organizations",
            "github_repositories",
            "github_repository_tags",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(
                    SELECT 1
                    FROM sqlite_master
                    WHERE type = 'table' AND name = ?1
                )",
                    [table],
                    |row| row.get(0),
                )
                .expect("table lookup should succeed");

            assert!(exists, "missing table: {table}",);
        }

        drop(connection);

        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn persists_inventory_settings() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        let expected = settings::InventorySettings {
            google_drive_enabled: true,
            automatic_updates: false,
            refresh_interval_hours: 12,
            full_reconciliation_days: 14,
            update_when_overdue_at_startup: false,
            directory_sheet_enabled: true,
            directory_sheet_url: "https://docs.google.com/spreadsheets/d/example/edit?gid=0"
                .to_string(),
            github_enabled: true,
            github_login: "octocat".to_string(),
            github_last_sync_at: String::new(),
            keeper_enabled: true,
            keeper_command: "keeper".to_string(),
            keeper_last_sync_at: String::new(),
            ..settings::InventorySettings::default()
        };

        settings::save(&database, &expected).expect("settings should save");

        let actual = settings::load(&database).expect("settings should load");

        assert_eq!(actual.automatic_updates, expected.automatic_updates,);
        assert_eq!(actual.google_drive_enabled, expected.google_drive_enabled);
        assert_eq!(
            actual.refresh_interval_hours,
            expected.refresh_interval_hours,
        );
        assert_eq!(actual.github_enabled, expected.github_enabled);
        assert_eq!(actual.github_login, expected.github_login);
        assert_eq!(
            actual.full_reconciliation_days,
            expected.full_reconciliation_days,
        );
        assert_eq!(
            actual.update_when_overdue_at_startup,
            expected.update_when_overdue_at_startup,
        );
        assert!(
            !settings::metadata_setup_skipped(&database).expect("metadata setup state should load")
        );
        settings::set_metadata_setup_skipped(&database, true)
            .expect("metadata setup state should save");
        assert!(
            settings::metadata_setup_skipped(&database)
                .expect("metadata setup state should reload")
        );
        assert!(
            !settings::bookmark_reminder_dismissed(&database)
                .expect("bookmark reminder state should default to visible")
        );
        settings::set_bookmark_reminder_dismissed(&database, true)
            .expect("bookmark reminder state should save");
        assert!(
            settings::bookmark_reminder_dismissed(&database)
                .expect("bookmark reminder state should reload")
        );

        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn deletes_tags_and_their_scopes() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        inventory::create_tag_with_scopes(
            &database,
            "Temporary Tag",
            "#369",
            &[inventory::TagScope::Directory],
        )
        .expect("tag should be created");

        inventory::delete_tag(&database, "temporary-tag").expect("tag should be deleted");

        assert!(
            inventory::list_tags(&database)
                .expect("tags should be readable")
                .iter()
                .all(|tag| tag.slug != "temporary-tag")
        );

        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn directory_csv_import_is_idempotent() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        let csv = b"email,name,status,type,organization\nformer@example.edu,Former User,former,person,ACEP\ngroup@example.edu,ACEP Staff,active,google group,ACEP\n";
        let first = directory::import_csv(&database, "directory.csv", csv)
            .expect("directory CSV should import");
        assert_eq!(first.rows_created, 2);
        assert_eq!(first.rows_rejected, 0);
        let second = directory::import_csv(&database, "directory.csv", csv)
            .expect("directory CSV should reimport");
        assert_eq!(second.rows_created, 0);
        assert_eq!(second.rows_updated, 2);
        let changed_csv = b"email,name,status,type,organization\nformer@example.edu,Former User,former,person,UAF\ngroup@example.edu,ACEP Staff,active,google group,\n";
        let third = directory::import_csv(&database, "directory.csv", changed_csv)
            .expect("updated directory CSV should reimport");
        assert_eq!(third.rows_updated, 2);
        let principals =
            directory::list_principals(&database).expect("directory identities should load");
        let former = principals
            .iter()
            .find(|principal| principal.primary_email == "former@example.edu")
            .expect("former identity should exist");
        let group = principals
            .iter()
            .find(|principal| principal.primary_email == "group@example.edu")
            .expect("group identity should exist");
        assert_eq!(former.organizations, "UAF");
        assert!(group.organizations.is_empty());
        let summary = directory::summary(&database).expect("summary should load");
        assert_eq!(summary.principals, 2);
        assert_eq!(summary.organizations, 2);
        assert_eq!(summary.groups, 1);
        assert_eq!(summary.former_or_departing, 1);
        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn directory_csv_import_applies_existing_person_tags_idempotently() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        let csv = b"email,name,tags\nformer@example.edu,Former User,Data Loss Risk; data-loss-risk\nactive@example.edu,Active User,\n";

        directory::import_csv(&database, "directory.csv", csv)
            .expect("directory CSV should import tags");
        directory::import_csv(&database, "directory.csv", csv)
            .expect("reimporting directory tags should be idempotent");

        let principal = directory::list_principals(&database)
            .expect("directory identities should load")
            .into_iter()
            .find(|principal| principal.primary_email == "former@example.edu")
            .expect("former identity should exist");
        assert_eq!(principal.tags.len(), 1);
        assert_eq!(principal.tags[0].slug, "data-loss-risk");

        let filtered = directory::list_principals_filtered(
            &database,
            "",
            "",
            "",
            "",
            "",
            "",
            "data-loss-risk",
        )
        .expect("directory should filter by tag");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].primary_email, "former@example.edu");

        let excluded = directory::list_principals_filtered(
            &database,
            "",
            "",
            "",
            "",
            "",
            "",
            "!data-loss-risk",
        )
        .expect("directory should exclude a tag");
        assert_eq!(excluded.len(), 1);
        assert_eq!(excluded[0].primary_email, "active@example.edu");
        let untagged = directory::list_principals_filtered(
            &database,
            "",
            "",
            "",
            "",
            "",
            "",
            inventory::UNTAGGED_TAG_FILTER,
        )
        .expect("directory should filter identities without tags");
        assert_eq!(untagged.len(), 1);
        assert_eq!(untagged[0].primary_email, "active@example.edu");

        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn directory_csv_preserves_custom_types_and_ignores_blank_type_updates() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        directory::import_csv(
            &database,
            "directory.csv",
            b"email,name,type,status\nresearcher@example.edu,Researcher,Affiliate Researcher,Active\nresearcher@example.edu,Researcher,,Active\n",
        )
        .expect("directory CSV should import");
        let principal = directory::list_principals(&database)
            .expect("directory should load")
            .into_iter()
            .find(|principal| principal.primary_email == "researcher@example.edu")
            .expect("researcher should exist");
        assert_eq!(principal.principal_type, "Affiliate Researcher");

        directory::import_csv(
            &database,
            "directory.csv",
            b"email,name,type,status\nresearcher@example.edu,Researcher,Senior Researcher,Active\n",
        )
        .expect("authoritative CSV type should update");
        let principal = directory::list_principals(&database)
            .expect("directory should load")
            .into_iter()
            .find(|principal| principal.primary_email == "researcher@example.edu")
            .expect("researcher should exist");
        assert_eq!(principal.principal_type, "Senior Researcher");
        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn shared_drive_inventories_are_isolated_and_discovery_is_reconciled() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        inventory::reconcile_shared_drives(
            &database,
            &[
                ("drive-a".to_string(), "Projects".to_string()),
                ("drive-b".to_string(), "Projects".to_string()),
            ],
        )
        .expect("Shared Drives should reconcile");
        let item = DriveItem {
            id: "file-a".to_string(),
            name: "Report.pdf".to_string(),
            path: "Reports/Report.pdf".to_string(),
            is_dir: false,
            size: 100,
            mime_type: "application/pdf".to_string(),
            mod_time: String::new(),
            metadata: BTreeMap::new(),
        };
        let scan_a = database
            .start_scan_run("shared-drive:drive-a")
            .expect("scan should start");
        inventory::synchronize_drive(
            &database,
            &inventory::shared_drive_scope("drive-a"),
            scan_a,
            &[item.clone()],
            false,
        )
        .expect("first Shared Drive should synchronize");
        let root_permissions = serde_json::from_str::<Vec<serde_json::Value>>(
            r#"[{"id":"manager-1","type":"user","role":"organizer","emailAddress":"manager@example.edu"},{"id":"viewer-1","type":"user","role":"reader","emailAddress":"viewer@example.edu"}]"#,
        )
        .expect("root permissions should parse");
        inventory::record_shared_drive_permissions(&database, "drive-a", &root_permissions)
            .expect("Shared Drive root permissions should synchronize");
        let mut second_item = item;
        second_item.id = "file-b".to_string();
        let scan_b = database
            .start_scan_run("shared-drive:drive-b")
            .expect("scan should start");
        inventory::synchronize_drive(
            &database,
            &inventory::shared_drive_scope("drive-b"),
            scan_b,
            &[second_item],
            false,
        )
        .expect("second Shared Drive should synchronize");
        assert_eq!(
            inventory::list_drive_directory(
                &database,
                &inventory::shared_drive_scope("drive-a"),
                Some("Reports"),
                "",
                "",
                "",
                "",
                "",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("first drive should list")
            .len(),
            1
        );
        assert_eq!(
            inventory::list_drive_directory(
                &database,
                &inventory::shared_drive_scope("drive-b"),
                Some("Reports"),
                "",
                "",
                "",
                "",
                "",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("second drive should list")
            .len(),
            1
        );
        inventory::reconcile_shared_drives(
            &database,
            &[("drive-a".to_string(), "Renamed Projects".to_string())],
        )
        .expect("Shared Drive discovery should reconcile again");
        let drives = inventory::list_shared_drives(&database).expect("Shared Drives should load");
        assert!(
            drives
                .iter()
                .find(|drive| drive.drive_id == "drive-a")
                .unwrap()
                .is_accessible
        );
        assert!(
            !drives
                .iter()
                .find(|drive| drive.drive_id == "drive-b")
                .unwrap()
                .is_accessible
        );
        assert_eq!(
            drives
                .iter()
                .find(|drive| drive.drive_id == "drive-a")
                .unwrap()
                .name,
            "Renamed Projects"
        );
        inventory::change_shared_drive_tags(
            &database,
            &["drive-a".to_string()],
            "to-delete",
            false,
        )
        .expect("Shared Drive tag should apply");
        let tagged_drives = inventory::list_shared_drives_filtered(
            &database,
            "renamed",
            "to-delete",
            "",
            "",
            "",
            "",
            "",
            "",
        )
        .expect("Shared Drives should filter by name and tag");
        assert_eq!(tagged_drives.len(), 1);
        assert_eq!(tagged_drives[0].drive_id, "drive-a");
        assert_eq!(tagged_drives[0].tags[0].slug, "to-delete");
        let untagged_drives = inventory::list_shared_drives_filtered(
            &database,
            "",
            "!to-delete",
            "",
            "",
            "",
            "",
            "",
            "",
        )
        .expect("Shared Drives should support negated tag filters");
        assert_eq!(untagged_drives.len(), 1);
        assert_eq!(untagged_drives[0].drive_id, "drive-b");
        assert_eq!(
            inventory::list_shared_drives_filtered(
                &database,
                "",
                inventory::UNTAGGED_TAG_FILTER,
                "",
                "",
                "",
                "",
                "",
                "",
            )
            .expect("Shared Drives should filter drives without tags")
            .len(),
            1,
        );
        let manager_filtered = inventory::list_shared_drives_filtered(
            &database,
            "",
            "",
            "",
            "",
            "",
            "",
            "manager@example",
            "",
        )
        .expect("Shared Drives should filter by manager");
        assert_eq!(manager_filtered.len(), 1);
        assert!(
            manager_filtered[0]
                .permission_identities
                .iter()
                .any(|identity| {
                    identity.label == "manager@example.edu"
                        && identity.roles.iter().any(|role| role == "organizer")
                })
        );
        inventory::change_shared_drive_tags(&database, &["drive-a".to_string()], "to-delete", true)
            .expect("Shared Drive tag should be removable");
        assert!(
            inventory::list_shared_drives_filtered(
                &database,
                "",
                "to-delete",
                "",
                "",
                "",
                "",
                "",
                "",
            )
            .expect("Shared Drive tag filter should load")
            .is_empty()
        );
        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn averages_the_five_most_recent_metadata_timings() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        for seconds in [10, 20, 30, 40, 50, 60] {
            database
                .record_metadata_timing("local-files", seconds)
                .expect("timing should save");
        }
        let estimate = database
            .metadata_timing_estimate("local-files")
            .expect("estimate should load")
            .expect("history should produce an estimate");
        assert_eq!(estimate.average_seconds, 40);
        assert_eq!(estimate.sample_count, 5);
        assert!(
            database
                .metadata_timing_estimate("unknown")
                .expect("missing estimate should load")
                .is_none()
        );
        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn manual_directory_entries_can_be_created_and_edited() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        let principal_id = directory::save_manual_principal(
            &database,
            None,
            "newuser",
            "new.user@example.edu",
            "New User",
            "staff",
            "active",
            "",
            "ACEP, UAF",
            "Created manually",
        )
        .expect("manual identity should be created");
        directory::save_manual_principal(
            &database,
            Some(principal_id),
            "newuser",
            "new.user@example.edu",
            "Updated User",
            "staff",
            "departing",
            "2026-12-31",
            "ACEP",
            "Updated manually",
        )
        .expect("manual identity should be updated");
        let principal = directory::get_principal(&database, principal_id)
            .expect("identity should load")
            .expect("identity should exist");
        assert_eq!(principal.display_name, "Updated User");
        assert_eq!(principal.username, "newuser");
        assert_eq!(principal.principal_type, "staff");
        assert_eq!(principal.status, "departing");
        assert_eq!(principal.departure_date, "2026-12-31");
        assert_eq!(principal.organizations, "ACEP");
        assert_eq!(principal.notes, "Updated manually");
        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn local_file_search_and_username_associations_span_the_inventory() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        local_files::synchronize(
            &database,
            &[
                crate::local_files::Item {
                    root_path: "/inventory".to_string(),
                    relative_path: "nested".to_string(),
                    name: "nested".to_string(),
                    extension: String::new(),
                    is_directory: true,
                    size_bytes: 42,
                    modified_unix: 1,
                    checksum_sha256: String::new(),
                    is_symlink: false,
                    symlink_target: String::new(),
                    owner_username: "jsmith".to_string(),
                    owner_identifier: "1001".to_string(),
                    group_name: "research".to_string(),
                    group_identifier: "2001".to_string(),
                },
                crate::local_files::Item {
                    root_path: "/inventory".to_string(),
                    relative_path: "nested/reports/Annual Report.pdf".to_string(),
                    name: "Annual Report.pdf".to_string(),
                    extension: "pdf".to_string(),
                    is_directory: false,
                    size_bytes: 42,
                    modified_unix: 1,
                    checksum_sha256: String::new(),
                    is_symlink: false,
                    symlink_target: String::new(),
                    owner_username: "jsmith".to_string(),
                    owner_identifier: "1001".to_string(),
                    group_name: "research".to_string(),
                    group_identifier: "2001".to_string(),
                },
            ],
        )
        .expect("local inventory should synchronize");

        let root_items = local_files::list_children(
            &database,
            "/inventory",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            false,
            "name",
            false,
        )
        .expect("root directory should load");
        assert_eq!(root_items[0].size_bytes, 42);

        let matches = local_files::list_children(
            &database,
            "/inventory",
            "",
            "annual report",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            false,
            "name",
            false,
        )
        .expect("recursive search should succeed");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].group_name, "research");
        assert_eq!(matches[0].owner_principal_id, 0);

        for (search, expected) in [
            ("PDF", 1),
            ("42 B", 2),
            ("jsmith", 2),
            ("research", 2),
            (matches[0].modified_label.as_str(), 2),
        ] {
            let broad_matches = local_files::list_children(
                &database,
                "/inventory",
                "",
                search,
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("full-tree detail search should succeed");
            assert_eq!(
                broad_matches.len(),
                expected,
                "search should match: {search}"
            );
        }

        for (label, item_type, owner, group, expected) in [
            ("type", "pdf", "", "", 1),
            ("owner", "", "jsmith", "", 1),
            ("group", "", "", "research", 1),
            ("combined", "pdf", "jsmith", "research", 1),
        ] {
            let filtered = local_files::list_children(
                &database,
                "/inventory",
                "nested/reports",
                "",
                "",
                "",
                item_type,
                "",
                "",
                owner,
                group,
                "",
                false,
                "name",
                false,
            )
            .expect("type, owner, and group filters should succeed");
            assert_eq!(
                filtered.len(),
                expected,
                "column filter should match: {label}"
            );
        }

        let principal_id = directory::save_manual_principal(
            &database,
            None,
            "jsmith",
            "jsmith@example.edu",
            "Jamie Smith",
            "person",
            "active",
            "",
            "",
            "",
        )
        .expect("person with a username should save");
        let associated = local_files::list_children(
            &database,
            "/inventory",
            "",
            "annual report",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            false,
            "name",
            false,
        )
        .expect("associated search should succeed");
        assert_eq!(associated[0].owner_principal_id, principal_id);
        assert_eq!(associated[0].owner_display_name, "Jamie Smith");
        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn inventory_upserts_and_soft_deletes_missing_items() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        let mut metadata = BTreeMap::new();
        metadata.insert("owner".to_string(), "owner@example.edu".to_string());
        metadata.insert(
            "permissions".to_string(),
            r#"[{"id":"permission-1","type":"user","role":"reader","emailAddress":"reader@example.edu"}]"#
                .to_string(),
        );
        let item = DriveItem {
            id: "drive-id-1".to_string(),
            name: "Report.pdf".to_string(),
            path: "Reports/Report.pdf".to_string(),
            is_dir: false,
            size: 42,
            mime_type: "application/pdf".to_string(),
            mod_time: "2026-08-29T12:00:00Z".to_string(),
            metadata,
        };
        let folder = DriveItem {
            id: "folder-id-1".to_string(),
            name: "Reports".to_string(),
            path: "Reports".to_string(),
            is_dir: true,
            size: -1,
            mime_type: "inode/directory".to_string(),
            mod_time: "2026-08-29T12:00:00Z".to_string(),
            metadata: BTreeMap::new(),
        };

        let first_scan = database
            .start_scan_run("my-drive")
            .expect("scan should start");
        let first_summary = inventory::synchronize_my_drive(
            &database,
            first_scan,
            &[folder.clone(), item.clone(), item.clone()],
            true,
        )
        .expect("inventory should synchronize");
        assert_eq!(first_summary.files_scanned, 1);
        assert_eq!(first_summary.folders_scanned, 1);
        assert_eq!(first_summary.bytes_discovered, 42);
        assert_eq!(first_summary.permissions_scanned, 1);
        let root_items = inventory::list_my_drive_directory(
            &database, None, "", "", "", "", "", "", false, "", "", "", false, "name", false,
        )
        .expect("explorer root should be readable");
        assert_eq!(root_items.len(), 1);
        assert_eq!(root_items[0].size_bytes, Some(42));
        let partial_scan = database
            .start_scan_run("partial:my-drive")
            .expect("partial scan should start");
        inventory::refresh_drive_items(
            &database,
            inventory::MY_DRIVE_SCOPE,
            partial_scan,
            std::slice::from_ref(&item),
        )
        .expect("selected item should refresh");
        assert_eq!(
            inventory::list_my_drive_directory(
                &database, None, "", "", "", "", "", "", false, "", "", "", false, "name", false,
            )
            .expect("partial refresh must preserve unselected items")
            .len(),
            1,
            "partial refresh must not mark an unselected folder deleted",
        );
        let connection = database.connect().expect("database should connect");
        connection
            .execute(
                "INSERT INTO migration_jobs
                 (source_scope, source_kind, status, phase, files_total, files_copied,
                  bytes_total, bytes_copied)
                 VALUES ('my-drive-ro', 'my-drive', 'running', 'Copying selected items',
                         4, 2, 100, 50)",
                [],
            )
            .expect("running migration should insert");
        let migration_id = connection.last_insert_rowid();
        assert_eq!(
            migration::active_count(&database).expect("active migrations should be countable"),
            1,
        );
        assert_eq!(
            migration::active_summary(&database)
                .expect("active migration progress should be readable"),
            migration::ActiveMigrationSummary {
                count: 1,
                files_total: 4,
                files_copied: 2,
                bytes_total: 100,
                bytes_copied: 50,
            },
        );
        connection
            .execute(
                "INSERT INTO migration_sources
                 (migration_id, item_id, name, relative_path, is_directory, status)
                 VALUES (?1, 'folder-id-1', 'Reports', 'Reports', 1, 'completed')",
                [migration_id],
            )
            .expect("completed migration source should insert");
        drop(connection);
        migration::complete_copy(&database, migration_id)
            .expect("completed copy should tag its source");
        assert_eq!(
            migration::active_count(&database).expect("completed migrations should be excluded"),
            0,
        );
        let migrated_items: i64 = database
            .connect()
            .expect("database should connect")
            .query_row(
                "SELECT COUNT(*) FROM drive_item_tags assignments
                 JOIN tags tag ON tag.id = assignments.tag_id
                 WHERE assignments.remote_name = 'my-drive-ro' AND tag.slug = 'migrated'",
                [],
                |row| row.get(0),
            )
            .expect("migrated assignments should be readable");
        assert_eq!(
            migrated_items, 2,
            "a migrated folder and its indexed descendants should be tagged",
        );
        let shared_scan = database
            .start_scan_run("shared-with-me")
            .expect("scan should start");
        let shared_summary = inventory::synchronize_drive(
            &database,
            inventory::SHARED_WITH_ME_SCOPE,
            shared_scan,
            &[folder.clone(), item.clone()],
            true,
        )
        .expect("Shared with me inventory should synchronize");
        assert_eq!(shared_summary.files_scanned, 1);
        assert_eq!(
            inventory::list_drive_directory(
                &database,
                inventory::SHARED_WITH_ME_SCOPE,
                Some("Reports"),
                "",
                "",
                "",
                "",
                "",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("Shared with me explorer should be readable")
            .len(),
            1,
        );
        assert_eq!(
            inventory::latest_summary_for(&database, "shared-with-me")
                .expect("Shared with me summary should be readable")
                .expect("Shared with me summary should exist")
                .files_scanned,
            1,
        );
        assert_eq!(
            inventory::list_my_drive_directory(
                &database, None, "report", "", "", "", "", "", false, "", "", "", false, "size",
                true,
            )
            .expect("filtered explorer root should be readable")
            .len(),
            1,
        );
        directory::import_csv(
            &database,
            "readers.csv",
            b"email,name,type,status\nreader@example.edu,Former Reader,person,former\n",
        )
        .expect("permission identity should import");
        inventory::create_tag(&database, "Former Staff", "#aa0000")
            .expect("permission identity tag should be created");
        let reader = directory::list_principals(&database)
            .expect("directory should be readable")
            .into_iter()
            .find(|principal| principal.primary_email == "reader@example.edu")
            .expect("permission identity should exist");
        directory::apply_principal_tag(&database, &[reader.id], "former-staff")
            .expect("permission identity tag should apply");
        let identity_state_items = inventory::list_my_drive_directory(
            &database,
            Some("Reports"),
            "",
            "",
            "",
            "",
            "",
            "",
            false,
            "",
            "",
            "",
            false,
            "name",
            false,
        )
        .expect("explorer identity states should be readable");
        assert!(!identity_state_items[0].owner_known);
        assert!(identity_state_items[0].permissions[0].known);
        assert!(
            inventory::list_my_drive_directory(
                &database,
                Some("Reports"),
                "",
                "",
                "",
                "",
                "",
                "",
                false,
                "",
                "former-staff",
                "",
                false,
                "name",
                false,
            )
            .expect("owner identity tag filter should be readable")
            .is_empty()
        );
        assert_eq!(
            inventory::list_my_drive_directory(
                &database,
                Some("Reports"),
                "",
                "",
                "",
                "",
                "",
                "",
                false,
                "",
                "",
                "former-staff",
                false,
                "name",
                false,
            )
            .expect("permission identity tag filter should be readable")
            .len(),
            1
        );
        assert_eq!(
            inventory::list_my_drive_directory(
                &database,
                Some("Reports"),
                "",
                "",
                "",
                ">40B",
                ">2026-01-01",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("size and modified expressions should be readable")
            .len(),
            1,
        );
        assert!(
            inventory::list_my_drive_directory(
                &database,
                Some("Reports"),
                "",
                "",
                "",
                ">5GB",
                "",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("large size expressions should be readable")
            .is_empty(),
        );
        assert!(
            inventory::list_my_drive_directory(
                &database,
                None,
                "",
                inventory::UNTAGGED_TAG_FILTER,
                "",
                "",
                "",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("My Drive should filter items without tags")
            .is_empty(),
        );
        assert!(
            inventory::list_my_drive_directory(
                &database, None, "missing", "", "", "", "", "", false, "", "", "", false, "name",
                false,
            )
            .expect("empty explorer search should be readable")
            .is_empty(),
        );
        assert_eq!(
            inventory::apply_tag_recursively(
                &database,
                &["folder-id-1".to_string()],
                "to-migrate",
            ).expect("recursive tag should apply"),
            2,
        );
        assert!(
            inventory::list_my_drive_directory(
                &database,
                None,
                "",
                "!to-migrate",
                "",
                "",
                "",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("My Drive should support negated tag filters")
            .is_empty(),
        );
        assert_eq!(
            inventory::remove_tag_recursively_for_scope(
                &database,
                inventory::MY_DRIVE_SCOPE,
                &["folder-id-1".to_string()],
                "to-migrate",
            )
            .expect("recursive tag should be removed"),
            2,
        );
        inventory::apply_tag_recursively(&database, &["folder-id-1".to_string()], "to-migrate")
            .expect("recursive tag should reapply");
        inventory::create_tag_with_description_and_scopes(
            &database,
            "Custom Review",
            "Content that needs an initial review",
            "#abcdef",
            &[inventory::TagScope::Directory],
        )
        .expect("custom tag should be created");
        inventory::update_tag_with_description_and_scopes(
            &database,
            "custom-review",
            "Review Soon",
            "Review this content before migration",
            "#159",
            &[inventory::TagScope::Directory],
        )
        .expect("custom tag should be editable");
        let custom_tag = inventory::list_tags(&database)
            .expect("tags should be readable")
            .into_iter()
            .find(|tag| tag.slug == "custom-review")
            .expect("custom tag should remain available");
        assert_eq!(custom_tag.name, "Review Soon");
        assert_eq!(
            custom_tag.description,
            "Review this content before migration"
        );
        assert_eq!(custom_tag.color, "#115599");
        assert!(custom_tag.directory);
        assert!(!custom_tag.my_drive);
        assert!(
            inventory::list_tags_for_scope(&database, inventory::TagScope::MyDrive)
                .expect("My Drive tags should be readable")
                .iter()
                .all(|tag| tag.slug != "custom-review")
        );
        assert!(
            inventory::apply_tag_recursively(
                &database,
                &["folder-id-1".to_string()],
                "custom-review",
            )
            .is_err(),
            "a Directory-only tag should not be applicable to My Drive",
        );
        directory::import_csv(
            &database,
            "owners.csv",
            b"email,name,type,status\nowner@example.edu,Owner,person,departing\n",
        )
        .expect("owner directory identity should import");
        let owner = directory::list_principals(&database)
            .expect("directory should be readable")
            .into_iter()
            .find(|principal| principal.primary_email == "owner@example.edu")
            .expect("owner identity should exist");
        assert!(
            inventory::list_my_drive_directory(
                &database,
                Some("Reports"),
                "",
                "",
                "",
                "",
                "",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("known owner state should be readable")[0]
                .owner_known
        );
        directory::apply_principal_tag(&database, &[owner.id], "custom-review")
            .expect("identity tag should apply");
        assert_eq!(
            directory::remove_principal_tag(&database, &[owner.id], "custom-review")
                .expect("identity tag should be removable"),
            1,
        );
        directory::apply_principal_tag(&database, &[owner.id], "custom-review")
            .expect("identity tag should reapply");
        assert_eq!(
            inventory::list_my_drive_directory(
                &database,
                Some("Reports"),
                "",
                "",
                "",
                "",
                "",
                "",
                false,
                "",
                "custom-review",
                "",
                false,
                "name",
                false,
            )
            .expect("identity-tagged owner should be searchable")
            .len(),
            1,
        );
        assert_eq!(
            inventory::list_my_drive_directory(
                &database,
                Some("Reports"),
                "",
                "",
                "",
                "",
                "",
                "jehaverlack",
                true,
                "reader@example.edu",
                "",
                "",
                false,
                "owner",
                false,
            )
            .expect("combined owner and permission filters should be readable")
            .len(),
            1,
        );
        assert_eq!(
            inventory::list_my_drive_directory(
                &database,
                Some("Reports"),
                "",
                "to-migrate",
                "",
                "",
                "",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("tagged folder contents should be readable")
            .len(),
            1,
        );
        let second_scan = database
            .start_scan_run("my-drive")
            .expect("scan should start");
        let summary = inventory::synchronize_my_drive(&database, second_scan, &[], true)
            .expect("empty authoritative inventory should synchronize");

        assert_eq!(summary.deleted_items, 2);
        assert_eq!(summary.files_scanned, 0);
        assert_eq!(summary.bytes_discovered, 0);
        assert!(
            inventory::list_my_drive_directory(
                &database,
                Some("Reports"),
                "",
                "",
                "",
                "",
                "",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("explorer directory should be readable")
            .is_empty(),
            "soft-deleted items must not appear in the explorer",
        );
        assert_eq!(
            inventory::list_drive_directory(
                &database,
                inventory::SHARED_WITH_ME_SCOPE,
                Some("Reports"),
                "",
                "",
                "",
                "",
                "",
                "",
                false,
                "",
                "",
                "",
                false,
                "name",
                false,
            )
            .expect("My Drive deletion must not affect Shared with me")
            .len(),
            1,
        );
        assert_eq!(
            inventory::list_my_drive_directory(
                &database, None, "", "", "", "", "", "", false, "", "", "", true, "name", false,
            )
            .expect("deleted explorer items should be optionally visible")
            .len(),
            1,
        );
        let connection = database.connect().expect("database should connect");
        let values: (i64, i64, String) = connection
            .query_row(
                "SELECT i.is_deleted, i.size_bytes, p.email_address
             FROM drive_items i JOIN drive_permissions p
               ON p.remote_name = i.remote_name AND p.item_id = i.item_id
             WHERE i.item_id = 'drive-id-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("soft-deleted item and permission should remain queryable");
        assert_eq!(values, (1, 42, "reader@example.edu".to_string()));
        let retained_tags: i64 = connection
            .query_row("SELECT COUNT(*) FROM drive_item_tags", [], |row| row.get(0))
            .expect("tags should remain queryable");
        assert_eq!(retained_tags, 4);

        drop(connection);
        let deleted_items = inventory::list_my_drive_directory(
            &database,
            None,
            "",
            inventory::DELETED_TAG_FILTER,
            "",
            "",
            "",
            "",
            false,
            "",
            "",
            "",
            true,
            "name",
            false,
        )
        .expect("deleted state filter should be readable");
        assert_eq!(deleted_items.len(), 1);
        assert!(deleted_items[0].is_deleted);
        let changed = inventory::apply_tag_recursively(
            &database,
            &["folder-id-1".to_string()],
            "needs-review",
        )
        .expect("deleted folder tags should be editable");
        assert_eq!(changed, 2);
        let retained_tags: i64 = database
            .connect()
            .expect("database should connect")
            .query_row("SELECT COUNT(*) FROM drive_item_tags", [], |row| row.get(0))
            .expect("deleted item tags should remain queryable");
        assert_eq!(retained_tags, 6);

        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn selected_missing_folder_soft_deletes_only_its_subtree() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        let items = [
            DriveItem {
                id: "missing-folder".to_string(),
                name: "Missing".to_string(),
                path: "Missing".to_string(),
                is_dir: true,
                size: -1,
                mime_type: "inode/directory".to_string(),
                mod_time: String::new(),
                metadata: BTreeMap::new(),
            },
            DriveItem {
                id: "missing-child".to_string(),
                name: "Child.txt".to_string(),
                path: "Missing/Child.txt".to_string(),
                is_dir: false,
                size: 10,
                mime_type: "text/plain".to_string(),
                mod_time: String::new(),
                metadata: BTreeMap::new(),
            },
            DriveItem {
                id: "retained-folder".to_string(),
                name: "Retained".to_string(),
                path: "Retained".to_string(),
                is_dir: true,
                size: -1,
                mime_type: "inode/directory".to_string(),
                mod_time: String::new(),
                metadata: BTreeMap::new(),
            },
        ];
        let initial_scan = database
            .start_scan_run("my-drive")
            .expect("scan should start");
        inventory::synchronize_my_drive(&database, initial_scan, &items, true)
            .expect("inventory should synchronize");
        let partial_scan = database
            .start_scan_run("partial:my-drive-ro")
            .expect("partial scan should start");
        let summary = inventory::mark_drive_item_missing(
            &database,
            inventory::MY_DRIVE_SCOPE,
            partial_scan,
            "missing-folder",
        )
        .expect("missing folder should be recorded");
        assert_eq!(summary.deleted_items, 2);
        let visible = inventory::list_my_drive_directory(
            &database, None, "", "", "", "", "", "", false, "", "", "", false, "name", false,
        )
        .expect("remaining root items should be readable");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].item_id, "retained-folder");
        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn enforces_migration_cancel_and_archive_lifecycle() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        let connection = database.connect().expect("database should connect");
        connection
            .execute(
                "INSERT INTO migration_jobs (source_scope, source_kind) VALUES ('my-drive', 'my-drive')",
                [],
            )
            .expect("draft migration should insert");
        let draft_id = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO migration_jobs
                 (source_scope, source_kind, status, phase, started_at, completed_at)
                 VALUES ('my-drive', 'my-drive', 'completed', 'Complete', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
                [],
            )
            .expect("completed migration should insert");
        let completed_id = connection.last_insert_rowid();
        drop(connection);

        migration::cancel(&database, draft_id).expect("unstarted migration should cancel");
        assert!(migration::cancel(&database, completed_id).is_err());
        assert!(
            migration::get(&database, draft_id)
                .expect("canceled migration lookup should succeed")
                .is_none(),
            "canceling should remove the migration and its history"
        );
        migration::archive(&database, completed_id).expect("completed migration should archive");
        assert!(migration::archive(&database, draft_id).is_err());

        let active = migration::list(&database, "", false, "created", true)
            .expect("active migrations should list");
        assert!(active.is_empty());
        let archived = migration::list(&database, "complete", true, "status", false)
            .expect("archived migrations should be searchable");
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, completed_id);
        assert!(!archived[0].archived_at.is_empty());

        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn local_download_migrations_are_recoverable_and_resumable() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");
        let connection = database.connect().expect("database should connect");
        connection
            .execute(
                "INSERT INTO migration_jobs
                 (source_scope, source_kind, operation_kind)
                 VALUES ('my-drive', 'my-drive', 'local-download')",
                [],
            )
            .expect("download migration should insert");
        let id = connection.last_insert_rowid();
        drop(connection);

        migration::set_local_destination(&database, id, "/tmp/boreal-download")
            .expect("local destination should save");
        migration::begin_copy(&database, id).expect("download should enter preflight");
        migration::confirm_copy_started(&database, id).expect("download should start");
        assert_eq!(
            migration::recover_interrupted(&database).expect("recovery should succeed"),
            1
        );
        let interrupted = migration::get(&database, id)
            .expect("migration should load")
            .expect("migration should exist");
        assert_eq!(interrupted.status, "interrupted");
        assert_eq!(interrupted.destination_kind, "local");
        assert_eq!(interrupted.destination_path, "/tmp/boreal-download");

        migration::begin_copy(&database, id).expect("interrupted download should resume");
        let resumed = migration::get(&database, id)
            .expect("migration should load")
            .expect("migration should exist");
        assert_eq!(resumed.status, "preflight");
        assert_eq!(resumed.resume_count, 1);

        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }

    #[test]
    fn tag_application_enforces_each_configured_scope() {
        let root = temporary_directory();
        let database = Database::initialize(&runtime(&root)).expect("database should initialize");

        let scopes = [
            ("Person Only", inventory::TagScope::Directory),
            ("My Drive Only", inventory::TagScope::MyDrive),
            ("Shared Drive Only", inventory::TagScope::SharedDrives),
            ("Shared With Me Only", inventory::TagScope::SharedWithMe),
            ("GitHub Only", inventory::TagScope::GitHubRepositories),
        ];
        for (name, scope) in scopes {
            inventory::create_tag_with_description_and_scopes(
                &database,
                name,
                "Scope enforcement test",
                "#123456",
                &[scope],
            )
            .expect("scoped tag should be created");
        }

        assert!(directory::apply_principal_tag(&database, &[1], "person-only").is_ok());
        assert!(
            directory::apply_principal_tag(&database, &[1], "my-drive-only").is_err(),
            "Persons must reject a My Drive-only tag"
        );

        assert!(
            inventory::apply_tag_recursively_for_scope(
                &database,
                inventory::MY_DRIVE_SCOPE,
                &["missing-item".to_string()],
                "my-drive-only",
            )
            .is_ok()
        );
        assert!(
            inventory::apply_tag_recursively_for_scope(
                &database,
                inventory::MY_DRIVE_SCOPE,
                &["missing-item".to_string()],
                "shared-with-me-only",
            )
            .is_err(),
            "My Drive must reject a Shared with me-only tag"
        );

        assert!(
            inventory::apply_tag_recursively_for_scope(
                &database,
                inventory::SHARED_WITH_ME_SCOPE,
                &["missing-item".to_string()],
                "shared-with-me-only",
            )
            .is_ok()
        );
        assert!(
            inventory::apply_tag_recursively_for_scope(
                &database,
                inventory::SHARED_WITH_ME_SCOPE,
                &["missing-item".to_string()],
                "shared-drive-only",
            )
            .is_err(),
            "Shared with me must reject a Shared Drive-only tag"
        );

        assert!(
            inventory::change_shared_drive_tags(
                &database,
                &["missing-drive".to_string()],
                "shared-drive-only",
                false,
            )
            .is_ok()
        );
        assert!(
            inventory::change_shared_drive_tags(
                &database,
                &["missing-drive".to_string()],
                "my-drive-only",
                false,
            )
            .is_err(),
            "Shared Drives must reject a My Drive-only tag"
        );

        let repository = crate::github::client::Repository {
            id: 42,
            name: "boreal".to_string(),
            full_name: "example/boreal".to_string(),
            html_url: "https://github.com/example/boreal".to_string(),
            description: Some("Repository inventory test".to_string()),
            owner: crate::github::client::Owner {
                id: 7,
                login: "example".to_string(),
                html_url: "https://github.com/example".to_string(),
                kind: "Organization".to_string(),
            },
            private: true,
            visibility: "private".to_string(),
            archived: false,
            disabled: false,
            fork: false,
            is_template: false,
            default_branch: "main".to_string(),
            language: Some("Rust".to_string()),
            topics: vec!["inventory".to_string()],
            size: 123,
            open_issues_count: 2,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-02-01T00:00:00Z".to_string(),
            pushed_at: "2026-02-01T00:00:00Z".to_string(),
            permissions: crate::github::client::RepositoryPermissions {
                pull: true,
                ..Default::default()
            },
        };
        github::synchronize(&database, &[repository]).expect("GitHub inventory should synchronize");
        assert!(
            github::change_tags(&database, &[42], "github-only", false).is_ok(),
            "GitHub repositories should accept GitHub tags"
        );
        assert!(
            github::change_tags(&database, &[42], "my-drive-only", false).is_err(),
            "GitHub repositories must reject My Drive-only tags"
        );
        let repositories = github::list(
            &database,
            "inventory",
            "example",
            "private",
            "read",
            "Rust",
            ">100 KB",
            ">2026-01-01",
            "github-only",
            false,
            "name",
            false,
        )
        .expect("GitHub repositories should be searchable");
        assert_eq!(repositories.len(), 1);
        assert_eq!(repositories[0].tags[0].slug, "github-only");
        let github_summary = github::summary(&database).expect("GitHub summary should load");
        assert_eq!(github_summary.repositories, 1);
        assert_eq!(github_summary.organizations, 1);
        assert_eq!(github_summary.private_repositories, 1);
        assert_eq!(github_summary.total_size_kb, 123);

        let keeper_folder = crate::keeper::client::SharedFolder {
            folder_uid: "keeper-folder-1".to_string(),
            name: "Operations".to_string(),
            folder_type: "Shared Folder".to_string(),
            folder_path: "/Operations".to_string(),
            access: vec![crate::keeper::client::FolderAccess {
                shared_to: "manager@example.edu".to_string(),
                permissions: "Can Manage Users".to_string(),
                target_kind: "user".to_string(),
            }],
        };
        keeper::synchronize(&database, &[keeper_folder])
            .expect("Keeper inventory should synchronize");
        let keeper_summary = keeper::summary(&database).expect("Keeper summary should load");
        assert_eq!(keeper_summary.shared_folders, 1);
        assert_eq!(keeper_summary.shared_with, 1);
        assert_eq!(keeper_summary.managed_folders, 1);

        fs::remove_dir_all(root).expect("temporary database directory should be removable");
    }
}
