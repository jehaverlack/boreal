use rusqlite::{Connection, params};

use super::DatabaseError;

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "foundation",
        sql: include_str!("migrations/0001_foundation.sql"),
    },
    Migration {
        version: 2,
        name: "drive_inventory",
        sql: include_str!("migrations/0002_drive_inventory.sql"),
    },
    Migration {
        version: 3,
        name: "folder_sizes",
        sql: include_str!("migrations/0003_folder_sizes.sql"),
    },
    Migration {
        version: 4,
        name: "tags",
        sql: include_str!("migrations/0004_tags.sql"),
    },
    Migration {
        version: 5,
        name: "tag_colors",
        sql: include_str!("migrations/0005_tag_colors.sql"),
    },
    Migration {
        version: 6,
        name: "identity_directory",
        sql: include_str!("migrations/0006_identity_directory.sql"),
    },
    Migration {
        version: 7,
        name: "identity_lookup_indexes",
        sql: include_str!("migrations/0007_identity_lookup_indexes.sql"),
    },
    Migration {
        version: 8,
        name: "principal_tags",
        sql: include_str!("migrations/0008_principal_tags.sql"),
    },
    Migration {
        version: 9,
        name: "shared_drives",
        sql: include_str!("migrations/0009_shared_drives.sql"),
    },
    Migration {
        version: 10,
        name: "manual_metadata_updates",
        sql: include_str!("migrations/0010_manual_metadata_updates.sql"),
    },
    Migration {
        version: 11,
        name: "directory_setup_choice",
        sql: include_str!("migrations/0011_directory_setup_choice.sql"),
    },
    Migration {
        version: 12,
        name: "safe_for_removal_tag",
        sql: include_str!("migrations/0012_safe_for_removal_tag.sql"),
    },
    Migration {
        version: 13,
        name: "shared_drive_tags",
        sql: include_str!("migrations/0013_shared_drive_tags.sql"),
    },
    Migration {
        version: 14,
        name: "shared_drive_permissions",
        sql: include_str!("migrations/0014_shared_drive_permissions.sql"),
    },
    Migration {
        version: 15,
        name: "tag_scopes",
        sql: include_str!("migrations/0015_tag_scopes.sql"),
    },
    Migration {
        version: 16,
        name: "builtin_tag_scopes",
        sql: include_str!("migrations/0016_builtin_tag_scopes.sql"),
    },
    Migration {
        version: 17,
        name: "builtin_tag_descriptions",
        sql: include_str!("migrations/0017_builtin_tag_descriptions.sql"),
    },
    Migration {
        version: 18,
        name: "default_tag_workflow",
        sql: include_str!("migrations/0018_default_tag_workflow.sql"),
    },
    Migration {
        version: 19,
        name: "remove_my_permissions_tag",
        sql: include_str!("migrations/0019_remove_my_permissions_tag.sql"),
    },
    Migration {
        version: 20,
        name: "migration_tracking",
        sql: include_str!("migrations/0020_migration_tracking.sql"),
    },
    Migration {
        version: 21,
        name: "migration_lifecycle",
        sql: include_str!("migrations/0021_migration_lifecycle.sql"),
    },
    Migration {
        version: 22,
        name: "migration_copy_progress",
        sql: include_str!("migrations/0022_migration_copy_progress.sql"),
    },
    Migration {
        version: 23,
        name: "migration_copy_completion",
        sql: include_str!("migrations/0023_migration_copy_completion.sql"),
    },
    Migration {
        version: 24,
        name: "remove_canceled_migrations",
        sql: include_str!("migrations/0024_remove_canceled_migrations.sql"),
    },
    Migration {
        version: 25,
        name: "migrated_tag",
        sql: include_str!("migrations/0025_migrated_tag.sql"),
    },
    Migration {
        version: 26,
        name: "transfer_migrations",
        sql: include_str!("migrations/0026_transfer_migrations.sql"),
    },
    Migration {
        version: 27,
        name: "keep_tag",
        sql: include_str!("migrations/0027_keep_tag.sql"),
    },
    Migration {
        version: 28,
        name: "github_inventory",
        sql: include_str!("migrations/0028_github_inventory.sql"),
    },
    Migration {
        version: 29,
        name: "keeper_inventory",
        sql: include_str!("migrations/0029_keeper_inventory.sql"),
    },
    Migration {
        version: 30,
        name: "local_file_inventory",
        sql: include_str!("migrations/0030_local_file_inventory.sql"),
    },
    Migration {
        version: 31,
        name: "local_file_tags",
        sql: include_str!("migrations/0031_local_file_tags.sql"),
    },
    Migration {
        version: 32,
        name: "s3_inventory",
        sql: include_str!("migrations/0032_s3_inventory.sql"),
    },
    Migration {
        version: 33,
        name: "local_file_ownership",
        sql: include_str!("migrations/0033_local_file_ownership.sql"),
    },
    Migration {
        version: 34,
        name: "local_file_details_and_tag_scopes",
        sql: include_str!("migrations/0034_local_file_details_and_tag_scopes.sql"),
    },
    Migration {
        version: 35,
        name: "metadata_timing_history",
        sql: include_str!("migrations/0035_metadata_timing_history.sql"),
    },
];

pub fn apply(connection: &mut Connection) -> Result<(), DatabaseError> {
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        ",
    )?;

    for migration in MIGRATIONS {
        let already_applied: bool = connection.query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM schema_migrations
                WHERE version = ?1
            )",
            [migration.version],
            |row| row.get(0),
        )?;

        if already_applied {
            continue;
        }

        let transaction = connection.transaction()?;

        transaction.execute_batch(migration.sql)?;

        transaction.execute(
            "INSERT INTO schema_migrations (
                version,
                name
            ) VALUES (?1, ?2)",
            params![migration.version, migration.name,],
        )?;

        transaction.commit()?;

        println!(
            "Applied database migration {}: {}",
            migration.version, migration.name,
        );
    }

    Ok(())
}
