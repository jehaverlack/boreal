use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::rclone::inventory::DriveItem;

use super::{Database, DatabaseError};

pub const MY_DRIVE_SCOPE: &str = "my-drive-ro";
pub const SHARED_WITH_ME_SCOPE: &str = "shared-with-me";
pub const SHARED_DRIVE_SCOPE_PREFIX: &str = "shared-drive:";
pub const DELETED_TAG_FILTER: &str = "__deleted__";
pub const UNTAGGED_TAG_FILTER: &str = "__untagged__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagScope {
    Directory,
    MyDrive,
    SharedDrives,
    SharedWithMe,
    GitHubRepositories,
    KeeperSharedFolders,
    LocalFiles,
}

impl TagScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::MyDrive => "my-drive",
            Self::SharedDrives => "shared-drives",
            Self::SharedWithMe => "shared-with-me",
            Self::GitHubRepositories => "github-repositories",
            Self::KeeperSharedFolders => "keeper-shared-folders",
            Self::LocalFiles => "local-files",
        }
    }

    pub fn for_inventory(inventory_scope: &str) -> Option<Self> {
        match inventory_scope {
            MY_DRIVE_SCOPE => Some(Self::MyDrive),
            SHARED_WITH_ME_SCOPE => Some(Self::SharedWithMe),
            value if value.starts_with(SHARED_DRIVE_SCOPE_PREFIX) => Some(Self::SharedDrives),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SharedDriveRow {
    pub drive_id: String,
    pub name: String,
    pub inventory_scope: String,
    pub is_accessible: bool,
    pub last_error: String,
    pub files_scanned: u64,
    pub folders_scanned: u64,
    pub permissions_scanned: u64,
    pub bytes_discovered: u64,
    pub modified_at: String,
    pub tags: Vec<Tag>,
    pub permission_identities: Vec<SharedDrivePermissionIdentity>,
}

#[derive(Debug, Clone)]
pub struct SharedDrivePermissionIdentity {
    pub label: String,
    pub roles: Vec<String>,
    pub known: bool,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredDriveFolder {
    pub item_id: String,
    pub name: String,
    pub modified_at: String,
}

pub fn record_migration_destination(
    database: &Database,
    drive_id: &str,
    drive_name: &str,
    folders: &[DiscoveredDriveFolder],
) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO scan_runs (scan_type, status, completed_at)
         VALUES ('migration-destination-discovery', 'completed', CURRENT_TIMESTAMP)",
        [],
    )?;
    let scan_id = transaction.last_insert_rowid();
    let inventory_scope = if drive_id.is_empty() {
        MY_DRIVE_SCOPE.to_string()
    } else {
        let scope = shared_drive_scope(drive_id);
        transaction.execute(
            "INSERT INTO shared_drives (drive_id, name, inventory_scope)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(drive_id) DO UPDATE SET name = excluded.name,
                 inventory_scope = excluded.inventory_scope, is_accessible = 1,
                 last_seen_at = CURRENT_TIMESTAMP",
            params![drive_id, drive_name, scope],
        )?;
        scope
    };
    let mut path_parts = Vec::new();
    for folder in folders {
        let parent_path = (!path_parts.is_empty()).then(|| path_parts.join("/"));
        path_parts.push(folder.name.clone());
        let relative_path = path_parts.join("/");
        transaction.execute(
            "INSERT INTO drive_items
             (remote_name, item_id, name, relative_path, parent_path, is_directory,
              mime_type, modified_at, metadata_json, last_seen_scan_id)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, 'application/vnd.google-apps.folder',
                     NULLIF(?6, ''), '{\"discovery\":\"migration-destination\"}', ?7)
             ON CONFLICT(remote_name, item_id) DO UPDATE SET
                name = excluded.name, relative_path = excluded.relative_path,
                parent_path = excluded.parent_path,
                modified_at = COALESCE(excluded.modified_at, drive_items.modified_at),
                last_seen_at = CURRENT_TIMESTAMP, is_deleted = 0, deleted_at = NULL",
            params![
                inventory_scope,
                folder.item_id,
                folder.name,
                relative_path,
                parent_path,
                folder.modified_at,
                scan_id,
            ],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub fn shared_drive_scope(drive_id: &str) -> String {
    format!("{SHARED_DRIVE_SCOPE_PREFIX}{drive_id}")
}

pub fn reconcile_shared_drives(
    database: &Database,
    drives: &[(String, String)],
) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute("UPDATE shared_drives SET is_accessible = 0", [])?;
    for (drive_id, name) in drives {
        transaction.execute(
            "INSERT INTO shared_drives (drive_id, name, inventory_scope)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(drive_id) DO UPDATE SET name = excluded.name,
                inventory_scope = excluded.inventory_scope, is_accessible = 1,
                last_seen_at = CURRENT_TIMESTAMP",
            params![drive_id, name, shared_drive_scope(drive_id)],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub fn record_shared_drive(
    database: &Database,
    drive_id: &str,
    name: &str,
) -> Result<(), DatabaseError> {
    let connection = database.connect()?;
    connection.execute(
        "INSERT INTO shared_drives (drive_id, name, inventory_scope)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(drive_id) DO UPDATE SET name = excluded.name,
            inventory_scope = excluded.inventory_scope, is_accessible = 1,
            last_seen_at = CURRENT_TIMESTAMP",
        params![drive_id, name, shared_drive_scope(drive_id)],
    )?;
    Ok(())
}

pub fn list_shared_drives(database: &Database) -> Result<Vec<SharedDriveRow>, DatabaseError> {
    list_shared_drives_filtered(database, "", "", "", "", "", "", "", "")
}

pub fn list_shared_drives_filtered(
    database: &Database,
    search: &str,
    tag_filter: &str,
    files_filter: &str,
    folders_filter: &str,
    size_filter: &str,
    modified_filter: &str,
    manager_filter: &str,
    permission_filter: &str,
) -> Result<Vec<SharedDriveRow>, DatabaseError> {
    let (files_comparison, files_value) = parse_count_filter(files_filter)?;
    let (folders_comparison, folders_value) = parse_count_filter(folders_filter)?;
    let (size_comparison, size_value) = parse_size_filter(size_filter)?;
    let (modified_comparison, modified_value) = parse_modified_filter(modified_filter)?;
    let (exclude_tag, tag_slug) = split_tag_filter(tag_filter);
    let connection = database.connect()?;
    let mut statement = connection.prepare(
        "SELECT sd.drive_id, sd.name, sd.inventory_scope, sd.is_accessible,
                COALESCE(last_error, ''),
                files_scanned, folders_scanned, permissions_scanned, bytes_discovered,
                COALESCE((SELECT MAX(di.modified_at) FROM drive_items di
                          WHERE di.remote_name = sd.inventory_scope AND di.is_deleted = 0), ''),
                (SELECT group_concat(t.slug || char(30) || t.name || char(30) || t.color || char(30) || t.description, char(31))
                 FROM shared_drive_tags sdt JOIN tags t ON t.id = sdt.tag_id
                 WHERE sdt.drive_id = sd.drive_id),
                (SELECT group_concat(label || char(30) || role, char(31)) FROM (
                    SELECT DISTINCT COALESCE(
                        NULLIF(p.email_address, ''), NULLIF(p.domain, ''),
                        NULLIF(p.display_name, ''), NULLIF(p.permission_type, ''), 'Unknown'
                    ) AS label, COALESCE(NULLIF(p.role, ''), 'unknown') AS role
                    FROM (
                        SELECT email_address, domain, display_name, permission_type, role
                        FROM drive_permissions WHERE remote_name = sd.inventory_scope
                        UNION ALL
                        SELECT email_address, domain, display_name, permission_type, role
                        FROM shared_drive_permissions WHERE drive_id = sd.drive_id
                    ) p
                    ORDER BY label COLLATE NOCASE, role COLLATE NOCASE
                ))
         FROM shared_drives sd
         WHERE (?1 = '' OR instr(lower(sd.name), lower(?1)) > 0
                    OR instr(lower(sd.drive_id), lower(?1)) > 0)
           AND (?2 = '' OR
                (?2 = '__untagged__' AND
                    ((?13 = 0 AND NOT EXISTS (
                        SELECT 1 FROM shared_drive_tags untagged_sdt
                        WHERE untagged_sdt.drive_id = sd.drive_id
                    )) OR
                    (?13 = 1 AND EXISTS (
                        SELECT 1 FROM shared_drive_tags untagged_sdt
                        WHERE untagged_sdt.drive_id = sd.drive_id
                    )))) OR
                (?2 <> '__untagged__' AND ?13 = 0 AND EXISTS (
                    SELECT 1 FROM shared_drive_tags filter_sdt
                    JOIN tags filter_tag ON filter_tag.id = filter_sdt.tag_id
                    WHERE filter_sdt.drive_id = sd.drive_id AND filter_tag.slug = ?2
                )) OR
                (?2 <> '__untagged__' AND ?13 = 1 AND NOT EXISTS (
                    SELECT 1 FROM shared_drive_tags filter_sdt
                    JOIN tags filter_tag ON filter_tag.id = filter_sdt.tag_id
                    WHERE filter_sdt.drive_id = sd.drive_id AND filter_tag.slug = ?2
                )))
           AND (?3 = 0 OR (?3 = 1 AND files_scanned > ?4) OR (?3 = 2 AND files_scanned >= ?4)
                OR (?3 = 3 AND files_scanned < ?4) OR (?3 = 4 AND files_scanned <= ?4) OR (?3 = 5 AND files_scanned = ?4))
           AND (?5 = 0 OR (?5 = 1 AND folders_scanned > ?6) OR (?5 = 2 AND folders_scanned >= ?6)
                OR (?5 = 3 AND folders_scanned < ?6) OR (?5 = 4 AND folders_scanned <= ?6) OR (?5 = 5 AND folders_scanned = ?6))
           AND (?7 = 0 OR (?7 = 1 AND bytes_discovered > ?8) OR (?7 = 2 AND bytes_discovered >= ?8)
                OR (?7 = 3 AND bytes_discovered < ?8) OR (?7 = 4 AND bytes_discovered <= ?8) OR (?7 = 5 AND bytes_discovered = ?8))
           AND (?9 = 0 OR
                (?9 = 1 AND substr(COALESCE((SELECT MAX(di.modified_at) FROM drive_items di WHERE di.remote_name = sd.inventory_scope AND di.is_deleted = 0), ''), 1, 10) > ?10) OR
                (?9 = 2 AND substr(COALESCE((SELECT MAX(di.modified_at) FROM drive_items di WHERE di.remote_name = sd.inventory_scope AND di.is_deleted = 0), ''), 1, 10) >= ?10) OR
                (?9 = 3 AND substr(COALESCE((SELECT MAX(di.modified_at) FROM drive_items di WHERE di.remote_name = sd.inventory_scope AND di.is_deleted = 0), ''), 1, 10) < ?10) OR
                (?9 = 4 AND substr(COALESCE((SELECT MAX(di.modified_at) FROM drive_items di WHERE di.remote_name = sd.inventory_scope AND di.is_deleted = 0), ''), 1, 10) <= ?10) OR
                (?9 = 5 AND substr(COALESCE((SELECT MAX(di.modified_at) FROM drive_items di WHERE di.remote_name = sd.inventory_scope AND di.is_deleted = 0), ''), 1, length(?10)) = ?10))
           AND (?11 = '' OR EXISTS (
                SELECT 1 FROM shared_drive_permissions manager_permission
                WHERE manager_permission.drive_id = sd.drive_id
                  AND lower(COALESCE(manager_permission.role, '')) IN ('organizer', 'owner')
                  AND instr(lower(COALESCE(NULLIF(manager_permission.email_address, ''),
                      NULLIF(manager_permission.domain, ''), NULLIF(manager_permission.display_name, ''),
                      NULLIF(manager_permission.permission_type, ''), 'Unknown')), lower(?11)) > 0
           ))
           AND (?12 = '' OR EXISTS (
                SELECT 1 FROM (
                    SELECT email_address, domain, display_name, permission_type
                    FROM drive_permissions WHERE remote_name = sd.inventory_scope
                    UNION ALL
                    SELECT email_address, domain, display_name, permission_type
                    FROM shared_drive_permissions WHERE drive_id = sd.drive_id
                ) identity_permission
                WHERE 1 = 1
                  AND instr(lower(COALESCE(NULLIF(identity_permission.email_address, ''),
                      NULLIF(identity_permission.domain, ''), NULLIF(identity_permission.display_name, ''),
                      NULLIF(identity_permission.permission_type, ''), 'Unknown')), lower(?12)) > 0
           ))
         ORDER BY is_accessible DESC, name COLLATE NOCASE, drive_id",
    )?;
    let mut drives = statement
        .query_map(
            params![
                search.trim(),
                tag_slug,
                files_comparison,
                files_value,
                folders_comparison,
                folders_value,
                size_comparison,
                size_value,
                modified_comparison,
                modified_value,
                manager_filter.trim(),
                permission_filter.trim(),
                exclude_tag,
            ],
            |row| {
                Ok(SharedDriveRow {
                    drive_id: row.get(0)?,
                    name: row.get(1)?,
                    inventory_scope: row.get(2)?,
                    is_accessible: row.get(3)?,
                    last_error: row.get(4)?,
                    files_scanned: row.get::<_, i64>(5)? as u64,
                    folders_scanned: row.get::<_, i64>(6)? as u64,
                    permissions_scanned: row.get::<_, i64>(7)? as u64,
                    bytes_discovered: row.get::<_, i64>(8)? as u64,
                    modified_at: row.get(9)?,
                    tags: row
                        .get::<_, Option<String>>(10)?
                        .map(|tags| {
                            tags.split('\u{1f}')
                                .filter_map(|tag| {
                                    let mut fields = tag.split('\u{1e}');
                                    Some(Tag {
                                        slug: fields.next()?.to_string(),
                                        name: fields.next()?.to_string(),
                                        color: fields.next()?.to_string(),
                                        description: fields.next()?.to_string(),
                                        directory: false,
                                        my_drive: false,
                                        shared_drives: false,
                                        shared_with_me: false,
                                        github_repositories: false,
                                        keeper_shared_folders: false,
                                        local_files: false,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    permission_identities: row
                        .get::<_, Option<String>>(11)?
                        .map(|identities| {
                            let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
                            for identity in identities.split('\u{1f}') {
                                if let Some((label, role)) = identity.split_once('\u{1e}') {
                                    let roles = grouped.entry(label.to_string()).or_default();
                                    if !roles.iter().any(|existing| existing == role) {
                                        roles.push(role.to_string());
                                    }
                                }
                            }
                            let mut identities = grouped
                                .into_iter()
                                .map(|(label, mut roles)| {
                                    roles.sort_by_key(|role| role.to_ascii_lowercase());
                                    SharedDrivePermissionIdentity {
                                        label,
                                        roles,
                                        known: false,
                                        tags: Vec::new(),
                                    }
                                })
                                .collect::<Vec<_>>();
                            identities.sort_by_key(|identity| identity.label.to_ascii_lowercase());
                            identities
                        })
                        .unwrap_or_default(),
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    let known_identities = connection
        .prepare(
            "SELECT lower(email) FROM principal_emails
             UNION SELECT lower(primary_email) FROM principals WHERE primary_email IS NOT NULL",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    let mut identity_tags: HashMap<String, Vec<Tag>> = HashMap::new();
    let mut tag_statement = connection.prepare(
        "SELECT lower(pe.email), t.slug, t.name, t.color, t.description
         FROM principal_emails pe
         JOIN principal_tags pt ON pt.principal_id = pe.principal_id
         JOIN tags t ON t.id = pt.tag_id
         ORDER BY t.name COLLATE NOCASE",
    )?;
    for row in tag_statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            Tag {
                slug: row.get(1)?,
                name: row.get(2)?,
                color: row.get(3)?,
                description: row.get(4)?,
                directory: false,
                my_drive: false,
                shared_drives: false,
                shared_with_me: false,
                github_repositories: false,
                keeper_shared_folders: false,
                local_files: false,
            },
        ))
    })? {
        let (email, tag) = row?;
        identity_tags.entry(email).or_default().push(tag);
    }
    for drive in &mut drives {
        for identity in &mut drive.permission_identities {
            let label = identity.label.to_ascii_lowercase();
            identity.known = known_identities.contains(&label);
            identity.tags = identity_tags.get(&label).cloned().unwrap_or_default();
        }
    }
    Ok(drives)
}

pub fn change_shared_drive_tags(
    database: &Database,
    drive_ids: &[String],
    tag_slug: &str,
    remove: bool,
) -> Result<usize, DatabaseError> {
    if drive_ids.is_empty() {
        return Err("Select at least one Shared Drive".into());
    }
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let tag_id: i64 = transaction
        .query_row(
            if remove {
                "SELECT id FROM tags WHERE slug = ?1"
            } else {
                "SELECT t.id FROM tags t JOIN tag_scopes s ON s.tag_id = t.id
                 WHERE t.slug = ?1 AND s.scope = 'shared-drives'"
            },
            [tag_slug],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| format!("Tag '{tag_slug}' is not available for Shared Drives"))?;
    let mut changed = 0;
    for drive_id in drive_ids {
        changed += if remove {
            transaction.execute(
                "DELETE FROM shared_drive_tags WHERE drive_id = ?1 AND tag_id = ?2",
                params![drive_id, tag_id],
            )?
        } else {
            transaction.execute(
                "INSERT OR IGNORE INTO shared_drive_tags (drive_id, tag_id)
                 SELECT drive_id, ?2 FROM shared_drives WHERE drive_id = ?1",
                params![drive_id, tag_id],
            )?
        };
    }
    transaction.commit()?;
    Ok(changed)
}

pub fn get_shared_drive(
    database: &Database,
    drive_id: &str,
) -> Result<Option<SharedDriveRow>, DatabaseError> {
    Ok(list_shared_drives(database)?
        .into_iter()
        .find(|drive| drive.drive_id == drive_id))
}

pub fn record_shared_drive_scan(
    database: &Database,
    drive_id: &str,
    summary: &InventorySummary,
) -> Result<(), DatabaseError> {
    let connection = database.connect()?;
    connection.execute(
        "UPDATE shared_drives SET last_scanned_at = CURRENT_TIMESTAMP, last_error = NULL,
                files_scanned = ?2, folders_scanned = ?3, permissions_scanned = ?4,
                bytes_discovered = ?5, deleted_items = ?6 WHERE drive_id = ?1",
        params![
            drive_id,
            summary.files_scanned as i64,
            summary.folders_scanned as i64,
            summary.permissions_scanned as i64,
            summary.bytes_discovered as i64,
            summary.deleted_items as i64
        ],
    )?;
    Ok(())
}

pub fn record_shared_drive_permissions(
    database: &Database,
    drive_id: &str,
    drive_permissions: &[Value],
) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "DELETE FROM shared_drive_permissions WHERE drive_id = ?1",
        [drive_id],
    )?;
    for permission in drive_permissions {
        transaction.execute(
            "INSERT INTO shared_drive_permissions (
                drive_id, permission_key, permission_id, permission_type, role,
                email_address, display_name, domain, raw_json, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CURRENT_TIMESTAMP)",
            params![
                drive_id,
                permission_key(&permission),
                field(&permission, "id"),
                field(&permission, "type"),
                field(&permission, "role"),
                field(&permission, "emailAddress"),
                field(&permission, "displayName"),
                field(&permission, "domain"),
                permission.to_string(),
            ],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub fn record_shared_drive_error(
    database: &Database,
    drive_id: &str,
    error: &str,
) -> Result<(), DatabaseError> {
    database.connect()?.execute(
        "UPDATE shared_drives SET last_error = ?2 WHERE drive_id = ?1",
        params![drive_id, error],
    )?;
    Ok(())
}

pub fn shared_drives_aggregate(database: &Database) -> Result<InventorySummary, DatabaseError> {
    let connection = database.connect()?;
    connection
        .query_row(
            "SELECT COALESCE(SUM(files_scanned), 0), COALESCE(SUM(folders_scanned), 0),
                COALESCE(SUM(permissions_scanned), 0), COALESCE(SUM(bytes_discovered), 0),
                COALESCE(SUM(deleted_items), 0)
         FROM shared_drives WHERE is_accessible = 1",
            [],
            |row| {
                Ok(InventorySummary {
                    files_scanned: row.get::<_, i64>(0)? as u64,
                    folders_scanned: row.get::<_, i64>(1)? as u64,
                    permissions_scanned: row.get::<_, i64>(2)? as u64,
                    bytes_discovered: row.get::<_, i64>(3)? as u64,
                    deleted_items: row.get::<_, i64>(4)? as u64,
                    completed_at: String::new(),
                })
            },
        )
        .map_err(Into::into)
}

#[derive(Debug, Clone, Default)]
pub struct InventorySummary {
    pub completed_at: String,
    pub files_scanned: u64,
    pub folders_scanned: u64,
    pub permissions_scanned: u64,
    pub bytes_discovered: u64,
    pub deleted_items: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct ScanTimingEstimate {
    pub elapsed_seconds: u64,
    pub average_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct DriveExplorerItem {
    pub item_id: String,
    pub name: String,
    pub relative_path: String,
    pub is_directory: bool,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub owner_email: Option<String>,
    pub owner_known: bool,
    pub owner_tags: Vec<Tag>,
    pub tags: Vec<Tag>,
    pub permissions: Vec<PermissionIdentity>,
    pub is_deleted: bool,
}

#[derive(Debug, Clone)]
pub struct DriveDownloadItem {
    pub relative_path: String,
    pub is_directory: bool,
}

pub fn get_drive_download_item(
    database: &Database,
    inventory_scope: &str,
    item_id: &str,
) -> Result<Option<DriveDownloadItem>, DatabaseError> {
    let connection = database.connect()?;
    connection
        .query_row(
            "SELECT relative_path, is_directory
             FROM drive_items WHERE remote_name = ?1 AND item_id = ?2 AND is_deleted = 0",
            params![inventory_scope, item_id],
            |row| {
                Ok(DriveDownloadItem {
                    relative_path: row.get(0)?,
                    is_directory: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

#[derive(Debug, Clone)]
pub struct PermissionIdentity {
    pub label: String,
    pub known: bool,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub color: String,
    pub directory: bool,
    pub my_drive: bool,
    pub shared_drives: bool,
    pub shared_with_me: bool,
    pub github_repositories: bool,
    pub keeper_shared_folders: bool,
    pub local_files: bool,
}

#[allow(dead_code)]
pub fn list_my_drive_directory(
    database: &Database,
    parent_path: Option<&str>,
    search: &str,
    tag_filter: &str,
    type_filter: &str,
    size_filter: &str,
    modified_filter: &str,
    owner_filter: &str,
    exclude_owner: bool,
    permission_filter: &str,
    owner_identity_tag_filter: &str,
    permission_identity_tag_filter: &str,
    include_deleted: bool,
    sort: &str,
    descending: bool,
) -> Result<Vec<DriveExplorerItem>, DatabaseError> {
    list_drive_directory(
        database,
        MY_DRIVE_SCOPE,
        parent_path,
        search,
        tag_filter,
        type_filter,
        size_filter,
        modified_filter,
        owner_filter,
        exclude_owner,
        permission_filter,
        owner_identity_tag_filter,
        permission_identity_tag_filter,
        include_deleted,
        sort,
        descending,
    )
}

pub fn list_drive_directory(
    database: &Database,
    inventory_scope: &str,
    parent_path: Option<&str>,
    search: &str,
    tag_filter: &str,
    type_filter: &str,
    size_filter: &str,
    modified_filter: &str,
    owner_filter: &str,
    exclude_owner: bool,
    permission_filter: &str,
    owner_identity_tag_filter: &str,
    permission_identity_tag_filter: &str,
    include_deleted: bool,
    sort: &str,
    descending: bool,
) -> Result<Vec<DriveExplorerItem>, DatabaseError> {
    let connection = database.connect()?;
    let (size_comparison, size_bytes) = parse_size_filter(size_filter)?;
    let (modified_comparison, modified_value) = parse_modified_filter(modified_filter)?;
    let (exclude_tag, tag_slug) = split_tag_filter(tag_filter);
    let remote = inventory_scope;
    let sort_expression = match sort {
        "type" => {
            "CASE WHEN is_directory THEN 'folder' ELSE COALESCE(mime_type, '') END COLLATE NOCASE"
        }
        "size" => {
            "COALESCE(CASE WHEN is_directory THEN cumulative_size_bytes ELSE size_bytes END, -1)"
        }
        "modified" => "COALESCE(modified_at, '')",
        "owner" => "COALESCE(owner_email, '') COLLATE NOCASE",
        _ => "name COLLATE NOCASE",
    };
    let direction = if descending { "DESC" } else { "ASC" };
    let directory_grouping = if sort == "name" {
        "is_directory DESC,"
    } else {
        ""
    };
    let sql = format!(
        "SELECT item_id, name, relative_path, is_directory, mime_type,
                CASE WHEN is_directory THEN cumulative_size_bytes ELSE size_bytes END,
                modified_at, owner_email,
                (SELECT group_concat(t.name || char(30) || t.color || char(30) || t.description, char(31))
                 FROM drive_item_tags dit JOIN tags t ON t.id = dit.tag_id
                 WHERE dit.remote_name = drive_items.remote_name
                   AND dit.item_id = drive_items.item_id),
                (SELECT group_concat(label, char(31)) FROM (
                    SELECT DISTINCT COALESCE(
                        NULLIF(p.email_address, ''), NULLIF(p.domain, ''),
                        NULLIF(p.display_name, ''), NULLIF(p.permission_type, ''), 'Unknown'
                    ) AS label
                    FROM drive_permissions p
                    WHERE p.remote_name = drive_items.remote_name
                      AND p.item_id = drive_items.item_id
                      AND COALESCE(p.email_address, '') <> COALESCE(drive_items.owner_email, '')
                    ORDER BY label COLLATE NOCASE
                )), is_deleted
         FROM drive_items
         WHERE remote_name = ?1
           AND (?13 = 1 OR is_deleted = 0)
           AND ((?2 IS NULL AND parent_path IS NULL) OR parent_path = ?2)
           AND (?3 = '' OR instr(lower(name), lower(?3)) > 0)
           AND (?4 = '' OR
                (?4 = '__deleted__' AND
                    ((?16 = 0 AND is_deleted = 1) OR (?16 = 1 AND is_deleted = 0))) OR
                (?4 = '__untagged__' AND
                    ((?16 = 0 AND NOT EXISTS (
                        SELECT 1 FROM drive_item_tags untagged_dit
                        WHERE untagged_dit.remote_name = drive_items.remote_name
                          AND untagged_dit.item_id = drive_items.item_id
                    )) OR
                    (?16 = 1 AND EXISTS (
                        SELECT 1 FROM drive_item_tags untagged_dit
                        WHERE untagged_dit.remote_name = drive_items.remote_name
                          AND untagged_dit.item_id = drive_items.item_id
                    )))) OR
                (?4 NOT IN ('__deleted__', '__untagged__') AND
                    ((?16 = 0 AND EXISTS (
                        SELECT 1 FROM drive_item_tags dit JOIN tags t ON t.id = dit.tag_id
                        WHERE dit.remote_name = drive_items.remote_name
                          AND dit.item_id = drive_items.item_id AND t.slug = ?4
                    )) OR
                    (?16 = 1 AND NOT EXISTS (
                        SELECT 1 FROM drive_item_tags dit JOIN tags t ON t.id = dit.tag_id
                        WHERE dit.remote_name = drive_items.remote_name
                          AND dit.item_id = drive_items.item_id AND t.slug = ?4
                    )))))
           AND (?5 = '' OR instr(lower(
                CASE WHEN is_directory THEN 'folder' ELSE COALESCE(mime_type, '') END
           ), lower(?5)) > 0)
           AND (?6 = 0 OR
                (?6 = 1 AND COALESCE(CASE WHEN is_directory THEN cumulative_size_bytes ELSE size_bytes END, 0) > ?7) OR
                (?6 = 2 AND COALESCE(CASE WHEN is_directory THEN cumulative_size_bytes ELSE size_bytes END, 0) >= ?7) OR
                (?6 = 3 AND COALESCE(CASE WHEN is_directory THEN cumulative_size_bytes ELSE size_bytes END, 0) < ?7) OR
                (?6 = 4 AND COALESCE(CASE WHEN is_directory THEN cumulative_size_bytes ELSE size_bytes END, 0) <= ?7) OR
                (?6 = 5 AND COALESCE(CASE WHEN is_directory THEN cumulative_size_bytes ELSE size_bytes END, 0) = ?7))
           AND (?8 = 0 OR
                (?8 = 1 AND substr(COALESCE(modified_at, ''), 1, 10) > ?9) OR
                (?8 = 2 AND substr(COALESCE(modified_at, ''), 1, 10) >= ?9) OR
                (?8 = 3 AND substr(COALESCE(modified_at, ''), 1, 10) < ?9) OR
                (?8 = 4 AND substr(COALESCE(modified_at, ''), 1, 10) <= ?9) OR
                (?8 = 5 AND substr(COALESCE(modified_at, ''), 1, length(?9)) = ?9))
           AND (?10 = '' OR
                (?11 = 0 AND instr(lower(COALESCE(owner_email, '')), lower(?10)) > 0) OR
                (?11 = 1 AND instr(lower(COALESCE(owner_email, '')), lower(?10)) = 0))
           AND (?12 = '' OR EXISTS (
                SELECT 1 FROM drive_permissions permission_filter
                WHERE permission_filter.remote_name = drive_items.remote_name
                  AND permission_filter.item_id = drive_items.item_id
                  AND instr(lower(
                      COALESCE(permission_filter.email_address, '') || ' ' ||
                      COALESCE(permission_filter.domain, '') || ' ' ||
                      COALESCE(permission_filter.display_name, '') || ' ' ||
                      COALESCE(permission_filter.permission_type, '')
                  ), lower(?12)) > 0
           ))
           AND (?14 = '' OR EXISTS (
                SELECT 1
                FROM principal_tags identity_pt
                JOIN tags identity_tag ON identity_tag.id = identity_pt.tag_id
                JOIN principals identity_principal ON identity_principal.id = identity_pt.principal_id
                WHERE identity_tag.slug = ?14
                  AND (lower(COALESCE(drive_items.owner_email, '')) = lower(COALESCE(identity_principal.primary_email, ''))
                       OR EXISTS (
                           SELECT 1 FROM principal_emails identity_alias
                           WHERE identity_alias.principal_id = identity_principal.id
                             AND lower(identity_alias.email) = lower(COALESCE(drive_items.owner_email, ''))
                       ))
           ))
           AND (?15 = '' OR EXISTS (
                SELECT 1
                FROM drive_permissions identity_permission
                JOIN principal_emails permission_email
                  ON lower(permission_email.email) = lower(COALESCE(identity_permission.email_address, ''))
                JOIN principal_tags permission_pt ON permission_pt.principal_id = permission_email.principal_id
                JOIN tags permission_tag ON permission_tag.id = permission_pt.tag_id
                WHERE identity_permission.remote_name = drive_items.remote_name
                  AND identity_permission.item_id = drive_items.item_id
                  AND permission_tag.slug = ?15
           ))
         ORDER BY {directory_grouping} {sort_expression} {direction}, name COLLATE NOCASE, item_id"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(
        params![
            remote,
            parent_path,
            search.trim(),
            tag_slug,
            type_filter.trim(),
            size_comparison,
            size_bytes,
            modified_comparison,
            modified_value,
            owner_filter.trim(),
            exclude_owner,
            permission_filter.trim(),
            include_deleted,
            owner_identity_tag_filter,
            permission_identity_tag_filter,
            exclude_tag,
        ],
        |row| {
            let size: Option<i64> = row.get(5)?;
            Ok(DriveExplorerItem {
                item_id: row.get(0)?,
                name: row.get(1)?,
                relative_path: row.get(2)?,
                is_directory: row.get(3)?,
                mime_type: row.get(4)?,
                size_bytes: size.map(|value| value as u64),
                modified_at: row.get(6)?,
                owner_email: row.get(7)?,
                owner_known: false,
                owner_tags: Vec::new(),
                tags: row
                    .get::<_, Option<String>>(8)?
                    .map(|tags| {
                        tags.split('\u{1f}')
                            .filter_map(|tag| {
                                let mut fields = tag.split('\u{1e}');
                                Some(Tag {
                                    slug: String::new(),
                                    name: fields.next()?.to_string(),
                                    color: fields.next()?.to_string(),
                                    description: fields.next()?.to_string(),
                                    directory: false,
                                    my_drive: false,
                                    shared_drives: false,
                                    shared_with_me: false,
                                    github_repositories: false,
                                    keeper_shared_folders: false,
                                    local_files: false,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                permissions: row
                    .get::<_, Option<String>>(9)?
                    .map(|permissions| {
                        permissions
                            .split('\u{1f}')
                            .map(|label| PermissionIdentity {
                                label: label.to_string(),
                                known: false,
                                tags: Vec::new(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                is_deleted: row.get(10)?,
            })
        },
    )?;

    let mut items = rows.collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut identity_tags: HashMap<String, Vec<Tag>> = HashMap::new();
    let known_identities = connection
        .prepare(
            "SELECT lower(email) FROM principal_emails
             UNION SELECT lower(primary_email) FROM principals WHERE primary_email IS NOT NULL",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    let mut tag_statement = connection.prepare(
        "SELECT lower(pe.email), t.slug, t.name, t.color, t.description
         FROM principal_emails pe
         JOIN principal_tags pt ON pt.principal_id = pe.principal_id
         JOIN tags t ON t.id = pt.tag_id
         ORDER BY t.name COLLATE NOCASE",
    )?;
    for row in tag_statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            Tag {
                slug: row.get(1)?,
                name: row.get(2)?,
                color: row.get(3)?,
                description: row.get(4)?,
                directory: false,
                my_drive: false,
                shared_drives: false,
                shared_with_me: false,
                github_repositories: false,
                keeper_shared_folders: false,
                local_files: false,
            },
        ))
    })? {
        let (email, tag) = row?;
        identity_tags.entry(email).or_default().push(tag);
    }
    for item in &mut items {
        if let Some(owner) = item.owner_email.as_ref() {
            let owner = owner.to_ascii_lowercase();
            item.owner_known = known_identities.contains(&owner);
            item.owner_tags = identity_tags.get(&owner).cloned().unwrap_or_default();
        }
        for permission in &mut item.permissions {
            let label = permission.label.to_ascii_lowercase();
            permission.known = known_identities.contains(&label);
            permission.tags = identity_tags.get(&label).cloned().unwrap_or_default();
        }
    }
    Ok(items)
}

fn comparison_prefix(value: &str) -> (i64, &str) {
    for (prefix, comparison) in [(">=", 2), ("<=", 4), (">", 1), ("<", 3), ("=", 5)] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return (comparison, rest.trim());
        }
    }
    (5, value.trim())
}

fn split_tag_filter(filter: &str) -> (bool, &str) {
    match filter.trim().strip_prefix('!') {
        Some(slug) => (true, slug.trim()),
        None => (false, filter.trim()),
    }
}

fn parse_count_filter(filter: &str) -> Result<(i64, i64), DatabaseError> {
    let filter = filter.trim();
    if filter.is_empty() {
        return Ok((0, 0));
    }
    let (comparison, value) = comparison_prefix(filter);
    let value = value
        .parse::<i64>()
        .map_err(|_| format!("Invalid count filter '{filter}'. Try >100 or <=25."))?;
    if value < 0 {
        return Err("Count filters cannot be negative".into());
    }
    Ok((comparison, value))
}

fn parse_size_filter(filter: &str) -> Result<(i64, i64), DatabaseError> {
    let filter = filter.trim();
    if filter.is_empty() {
        return Ok((0, 0));
    }
    let (mut comparison, value) = comparison_prefix(filter);
    if !matches!(filter.chars().next(), Some('>' | '<' | '=')) {
        comparison = 2; // A bare size remains a convenient minimum-size filter.
    }
    let split = value
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(value.len());
    let number = value[..split]
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("Invalid size filter '{filter}'. Try >5GB or <=250MB."))?;
    let unit = value[split..].trim().to_ascii_uppercase();
    let multiplier = match unit.as_str() {
        "" | "MB" => 1_000_000_f64,
        "B" => 1_f64,
        "KB" => 1_000_f64,
        "GB" => 1_000_000_000_f64,
        "TB" => 1_000_000_000_000_f64,
        "KIB" => 1_024_f64,
        "MIB" => 1_048_576_f64,
        "GIB" => 1_073_741_824_f64,
        "TIB" => 1_099_511_627_776_f64,
        _ => {
            return Err(
                format!("Unknown size unit in '{filter}'. Use B, KB, MB, GB, or TB.").into(),
            );
        }
    };
    let bytes = number * multiplier;
    if !bytes.is_finite() || bytes < 0.0 || bytes > i64::MAX as f64 {
        return Err(format!("Size filter '{filter}' is out of range.").into());
    }
    Ok((comparison, bytes.round() as i64))
}

fn parse_modified_filter(filter: &str) -> Result<(i64, String), DatabaseError> {
    let filter = filter.trim();
    if filter.is_empty() {
        return Ok((0, String::new()));
    }
    let (comparison, value) = comparison_prefix(filter);
    let valid = value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| index == 4 || index == 7 || character.is_ascii_digit());
    if !valid {
        return Err(format!("Invalid modified-date filter '{filter}'. Try >2026-01-01.").into());
    }
    Ok((comparison, value.to_string()))
}

pub fn list_tags(database: &Database) -> Result<Vec<Tag>, DatabaseError> {
    list_tags_query(database, None)
}

pub fn list_tags_for_scope(
    database: &Database,
    scope: TagScope,
) -> Result<Vec<Tag>, DatabaseError> {
    list_tags_query(database, Some(scope))
}

fn list_tags_query(
    database: &Database,
    scope: Option<TagScope>,
) -> Result<Vec<Tag>, DatabaseError> {
    let connection = database.connect()?;
    let mut statement = connection.prepare(
        "SELECT t.slug, t.name, t.description, t.color,
                EXISTS(SELECT 1 FROM tag_scopes s WHERE s.tag_id = t.id AND s.scope = 'directory'),
                EXISTS(SELECT 1 FROM tag_scopes s WHERE s.tag_id = t.id AND s.scope = 'my-drive'),
                EXISTS(SELECT 1 FROM tag_scopes s WHERE s.tag_id = t.id AND s.scope = 'shared-drives'),
                EXISTS(SELECT 1 FROM tag_scopes s WHERE s.tag_id = t.id AND s.scope = 'shared-with-me'),
                EXISTS(SELECT 1 FROM tag_scopes s WHERE s.tag_id = t.id AND s.scope = 'github-repositories'),
                EXISTS(SELECT 1 FROM tag_scopes s WHERE s.tag_id = t.id AND s.scope = 'keeper-shared-folders'),
                EXISTS(SELECT 1 FROM tag_scopes s WHERE s.tag_id = t.id AND s.scope = 'local-files')
         FROM tags t
         WHERE ?1 IS NULL OR EXISTS(
             SELECT 1 FROM tag_scopes selected
             WHERE selected.tag_id = t.id AND selected.scope = ?1
         )
         ORDER BY t.name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([scope.map(TagScope::as_str)], |row| {
        Ok(Tag {
            slug: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            color: row.get(3)?,
            directory: row.get(4)?,
            my_drive: row.get(5)?,
            shared_drives: row.get(6)?,
            shared_with_me: row.get(7)?,
            github_repositories: row.get(8)?,
            keeper_shared_folders: row.get(9)?,
            local_files: row.get(10)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

#[allow(dead_code)]
pub fn create_tag(database: &Database, name: &str, color: &str) -> Result<(), DatabaseError> {
    create_tag_with_scopes(database, name, color, &TagScope::ALL)
}

impl TagScope {
    #[allow(dead_code)]
    const ALL: [Self; 7] = [
        Self::Directory,
        Self::MyDrive,
        Self::SharedDrives,
        Self::SharedWithMe,
        Self::GitHubRepositories,
        Self::KeeperSharedFolders,
        Self::LocalFiles,
    ];
}

pub fn update_tag_scopes_bulk(
    database: &Database,
    updates: &[(String, Vec<TagScope>)],
) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    for (slug, scopes) in updates {
        validate_tag_scopes(scopes)?;
        let tag_id: i64 = transaction
            .query_row("SELECT id FROM tags WHERE slug=?1", [slug], |row| {
                row.get(0)
            })
            .map_err(|_| format!("Unknown tag: {slug}"))?;
        transaction.execute("DELETE FROM tag_scopes WHERE tag_id=?1", [tag_id])?;
        save_tag_scopes(&transaction, tag_id, scopes)?;
    }
    transaction.commit()?;
    Ok(())
}

pub fn create_tag_with_scopes(
    database: &Database,
    name: &str,
    color: &str,
    scopes: &[TagScope],
) -> Result<(), DatabaseError> {
    create_tag_with_description_and_scopes(database, name, "", color, scopes)
}

pub fn create_tag_with_description_and_scopes(
    database: &Database,
    name: &str,
    description: &str,
    color: &str,
    scopes: &[TagScope],
) -> Result<(), DatabaseError> {
    let color = validate_tag(name, color)?;
    let description = validate_tag_description(description)?;
    validate_tag_scopes(scopes)?;
    let slug = tag_slug(name);
    if slug.is_empty() {
        return Err("Tag name must contain letters or numbers".into());
    }
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO tags (slug, name, description, color) VALUES (?1, ?2, ?3, ?4)",
        params![slug, name.trim(), description, color],
    )?;
    let tag_id = transaction.last_insert_rowid();
    save_tag_scopes(&transaction, tag_id, scopes)?;
    transaction.commit()?;
    Ok(())
}

#[allow(dead_code)]
pub fn update_tag(
    database: &Database,
    slug: &str,
    name: &str,
    color: &str,
) -> Result<(), DatabaseError> {
    update_tag_with_scopes(database, slug, name, color, &TagScope::ALL)
}

pub fn update_tag_with_scopes(
    database: &Database,
    slug: &str,
    name: &str,
    color: &str,
    scopes: &[TagScope],
) -> Result<(), DatabaseError> {
    update_tag_with_description_and_scopes(database, slug, name, "", color, scopes)
}

pub fn update_tag_with_description_and_scopes(
    database: &Database,
    slug: &str,
    name: &str,
    description: &str,
    color: &str,
    scopes: &[TagScope],
) -> Result<(), DatabaseError> {
    let color = validate_tag(name, color)?;
    let description = validate_tag_description(description)?;
    validate_tag_scopes(scopes)?;
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let updated = transaction.execute(
        "UPDATE tags SET name = ?2, description = ?3, color = ?4 WHERE slug = ?1",
        params![slug, name.trim(), description, color],
    )?;
    if updated == 0 {
        return Err(format!("Unknown tag: {slug}").into());
    }
    let tag_id = transaction.query_row("SELECT id FROM tags WHERE slug = ?1", [slug], |row| {
        row.get(0)
    })?;
    transaction.execute("DELETE FROM tag_scopes WHERE tag_id = ?1", [tag_id])?;
    save_tag_scopes(&transaction, tag_id, scopes)?;
    transaction.commit()?;
    Ok(())
}

pub fn delete_tag(database: &Database, slug: &str) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let deleted = transaction.execute("DELETE FROM tags WHERE slug = ?1", [slug])?;
    if deleted == 0 {
        return Err(format!("Unknown tag: {slug}").into());
    }
    transaction.commit()?;
    Ok(())
}

fn validate_tag_scopes(scopes: &[TagScope]) -> Result<(), DatabaseError> {
    if scopes.is_empty() {
        return Err("Select at least one tag location".into());
    }
    Ok(())
}

fn save_tag_scopes(
    transaction: &rusqlite::Transaction<'_>,
    tag_id: i64,
    scopes: &[TagScope],
) -> Result<(), DatabaseError> {
    for scope in scopes {
        transaction.execute(
            "INSERT OR IGNORE INTO tag_scopes (tag_id, scope) VALUES (?1, ?2)",
            params![tag_id, scope.as_str()],
        )?;
    }
    Ok(())
}

fn validate_tag(name: &str, color: &str) -> Result<String, DatabaseError> {
    if name.trim().is_empty() || name.trim().len() > 40 {
        return Err("Tag name must be between 1 and 40 characters".into());
    }

    let color = color.trim();
    if !color.starts_with('#') || !color[1..].bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Tag color must use #RGB or #RRGGBB format".into());
    }

    match color.len() {
        4 => {
            let mut normalized = String::with_capacity(7);
            normalized.push('#');
            for digit in color[1..].chars() {
                normalized.push(digit);
                normalized.push(digit);
            }
            Ok(normalized.to_ascii_lowercase())
        }
        7 => Ok(color.to_ascii_lowercase()),
        _ => Err("Tag color must use #RGB or #RRGGBB format".into()),
    }
}

fn validate_tag_description(description: &str) -> Result<&str, DatabaseError> {
    let description = description.trim();
    if description.len() > 500 {
        return Err("Tag description must not exceed 500 characters".into());
    }
    Ok(description)
}

fn tag_slug(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .chars()
        .fold(String::new(), |mut slug, character| {
            if character.is_ascii_alphanumeric() {
                slug.push(character);
            } else if !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
            slug
        })
        .trim_end_matches('-')
        .to_string()
}

#[allow(dead_code)]
pub fn apply_tag_recursively(
    database: &Database,
    item_ids: &[String],
    tag_slug: &str,
) -> Result<usize, DatabaseError> {
    apply_tag_recursively_for_scope(database, MY_DRIVE_SCOPE, item_ids, tag_slug)
}

pub fn apply_tag_recursively_for_scope(
    database: &Database,
    inventory_scope: &str,
    item_ids: &[String],
    tag_slug: &str,
) -> Result<usize, DatabaseError> {
    if item_ids.is_empty() {
        return Err("Select at least one Drive item".into());
    }
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let tag_scope = TagScope::for_inventory(inventory_scope)
        .ok_or_else(|| format!("Unknown inventory scope: {inventory_scope}"))?;
    let tag_id: i64 = transaction
        .query_row(
            "SELECT t.id FROM tags t JOIN tag_scopes s ON s.tag_id = t.id
             WHERE t.slug = ?1 AND s.scope = ?2",
            params![tag_slug, tag_scope.as_str()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| format!("Tag '{tag_slug}' is not available in this explorer"))?;
    let remote = inventory_scope;
    let mut applied = 0;

    for item_id in item_ids {
        applied += transaction.execute(
            "INSERT OR IGNORE INTO drive_item_tags (remote_name, item_id, tag_id)
             SELECT descendant.remote_name, descendant.item_id, ?3
             FROM drive_items selected
             JOIN drive_items descendant
               ON descendant.remote_name = selected.remote_name
              AND descendant.is_deleted = selected.is_deleted
              AND (
                   descendant.item_id = selected.item_id
                   OR (selected.is_directory = 1 AND
                       substr(descendant.relative_path, 1, length(selected.relative_path) + 1)
                           = selected.relative_path || '/')
              )
             WHERE selected.remote_name = ?1
               AND selected.item_id = ?2",
            params![remote, item_id, tag_id],
        )?;
    }
    transaction.commit()?;
    Ok(applied)
}

pub fn remove_tag_recursively_for_scope(
    database: &Database,
    inventory_scope: &str,
    item_ids: &[String],
    tag_slug: &str,
) -> Result<usize, DatabaseError> {
    if item_ids.is_empty() {
        return Err("Select at least one Drive item".into());
    }
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let tag_id: i64 = transaction
        .query_row("SELECT id FROM tags WHERE slug = ?1", [tag_slug], |row| {
            row.get(0)
        })
        .optional()?
        .ok_or_else(|| format!("Unknown tag: {tag_slug}"))?;
    let mut removed = 0;
    for item_id in item_ids {
        removed += transaction.execute(
            "DELETE FROM drive_item_tags
             WHERE tag_id = ?3 AND remote_name = ?1 AND item_id IN (
                SELECT descendant.item_id
                FROM drive_items selected
                JOIN drive_items descendant
                  ON descendant.remote_name = selected.remote_name
                 AND descendant.is_deleted = selected.is_deleted
                 AND (descendant.item_id = selected.item_id
                      OR (selected.is_directory = 1 AND
                          substr(descendant.relative_path, 1, length(selected.relative_path) + 1)
                              = selected.relative_path || '/'))
                WHERE selected.remote_name = ?1 AND selected.item_id = ?2
             )",
            params![inventory_scope, item_id, tag_id],
        )?;
    }
    transaction.commit()?;
    Ok(removed)
}

pub fn synchronize_my_drive(
    database: &Database,
    scan_id: i64,
    items: &[DriveItem],
    include_permissions: bool,
) -> Result<InventorySummary, DatabaseError> {
    synchronize_drive(
        database,
        MY_DRIVE_SCOPE,
        scan_id,
        items,
        include_permissions,
    )
}

pub fn synchronize_drive(
    database: &Database,
    inventory_scope: &str,
    scan_id: i64,
    items: &[DriveItem],
    include_permissions: bool,
) -> Result<InventorySummary, DatabaseError> {
    synchronize_drive_inner(
        database,
        inventory_scope,
        scan_id,
        items,
        include_permissions,
        true,
    )
}

pub fn refresh_drive_items(
    database: &Database,
    inventory_scope: &str,
    scan_id: i64,
    items: &[DriveItem],
) -> Result<InventorySummary, DatabaseError> {
    synchronize_drive_inner(database, inventory_scope, scan_id, items, true, false)
}

pub fn mark_drive_item_missing(
    database: &Database,
    inventory_scope: &str,
    scan_id: i64,
    item_id: &str,
) -> Result<InventorySummary, DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let (relative_path, is_directory): (String, bool) = transaction.query_row(
        "SELECT relative_path, is_directory FROM drive_items
         WHERE remote_name = ?1 AND item_id = ?2 AND is_deleted = 0",
        params![inventory_scope, item_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let child_prefix = format!("{relative_path}/%");
    let deleted_items = transaction.execute(
        "UPDATE drive_items
         SET is_deleted = 1, deleted_at = COALESCE(deleted_at, CURRENT_TIMESTAMP)
         WHERE remote_name = ?1 AND is_deleted = 0
           AND (item_id = ?2 OR (?3 = 1 AND relative_path LIKE ?4))",
        params![inventory_scope, item_id, is_directory, child_prefix],
    )? as u64;

    let mut folder_sizes: HashMap<String, u64> = HashMap::new();
    {
        let mut statement = transaction.prepare(
            "SELECT relative_path, COALESCE(size_bytes, 0) FROM drive_items
             WHERE remote_name = ?1 AND is_deleted = 0 AND is_directory = 0",
        )?;
        let rows = statement.query_map([inventory_scope], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (path, size) = row?;
            let mut ancestor = path.rsplit_once('/').map(|(parent, _)| parent);
            while let Some(folder_path) = ancestor {
                let total = folder_sizes.entry(folder_path.to_string()).or_default();
                *total = total.saturating_add(size.max(0) as u64);
                ancestor = folder_path.rsplit_once('/').map(|(parent, _)| parent);
            }
        }
    }
    transaction.execute(
        "UPDATE drive_items SET cumulative_size_bytes = 0
         WHERE remote_name = ?1 AND is_directory = 1",
        [inventory_scope],
    )?;
    for (folder_path, size) in folder_sizes {
        transaction.execute(
            "UPDATE drive_items SET cumulative_size_bytes = ?3
             WHERE remote_name = ?1 AND relative_path = ?2 AND is_directory = 1",
            params![inventory_scope, folder_path, size as i64],
        )?;
    }

    transaction.execute(
        "UPDATE scan_runs SET status = 'complete', completed_at = CURRENT_TIMESTAMP,
             error_message = NULL, deleted_items = ?2 WHERE id = ?1",
        params![scan_id, deleted_items as i64],
    )?;
    let completed_at = transaction.query_row(
        "SELECT completed_at FROM scan_runs WHERE id = ?1",
        [scan_id],
        |row| row.get(0),
    )?;
    transaction.commit()?;
    Ok(InventorySummary {
        deleted_items,
        completed_at,
        ..InventorySummary::default()
    })
}

fn synchronize_drive_inner(
    database: &Database,
    inventory_scope: &str,
    scan_id: i64,
    items: &[DriveItem],
    include_permissions: bool,
    authoritative: bool,
) -> Result<InventorySummary, DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let remote = inventory_scope;
    let mut summary = InventorySummary::default();
    let mut seen_item_ids = HashSet::new();
    let mut folder_sizes: HashMap<&str, u64> = HashMap::new();

    for item in items {
        // Rclone may expose the same Drive object through multiple paths,
        // particularly shortcuts and legacy multi-parent objects. Drive ID is
        // authoritative, so count and store each object only once per scan.
        if !seen_item_ids.insert(item.id.as_str()) {
            continue;
        }

        let size = (item.size >= 0).then_some(item.size);
        let owner = item.metadata.get("owner").map(String::as_str);
        let created = item.metadata.get("btime").map(String::as_str);
        let parent_path = item.path.rsplit_once('/').map(|(parent, _)| parent);
        let metadata_json = serde_json::to_string(&item.metadata)?;

        transaction.execute(
            "INSERT INTO drive_items (
                remote_name, item_id, name, relative_path, parent_path,
                is_directory, mime_type, size_bytes, modified_at, created_at,
                owner_email, metadata_json, last_seen_scan_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(remote_name, item_id) DO UPDATE SET
                name = excluded.name,
                relative_path = excluded.relative_path,
                parent_path = excluded.parent_path,
                is_directory = excluded.is_directory,
                mime_type = excluded.mime_type,
                size_bytes = excluded.size_bytes,
                modified_at = excluded.modified_at,
                created_at = excluded.created_at,
                owner_email = excluded.owner_email,
                metadata_json = excluded.metadata_json,
                last_seen_at = CURRENT_TIMESTAMP,
                last_seen_scan_id = excluded.last_seen_scan_id,
                is_deleted = 0,
                deleted_at = NULL",
            params![
                remote,
                item.id,
                item.name,
                item.path,
                parent_path,
                item.is_dir,
                empty_as_none(&item.mime_type),
                size,
                empty_as_none(&item.mod_time),
                created,
                owner,
                metadata_json,
                scan_id,
            ],
        )?;

        if include_permissions {
            transaction.execute(
                "DELETE FROM drive_permissions WHERE remote_name = ?1 AND item_id = ?2",
                params![remote, item.id],
            )?;

            for permission in permissions(item)? {
                let key = permission_key(&permission);
                transaction.execute(
                    "INSERT INTO drive_permissions (
                    remote_name, item_id, permission_key, permission_id,
                    permission_type, role, email_address, display_name,
                    domain, raw_json, last_seen_scan_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        remote,
                        item.id,
                        key,
                        field(&permission, "id"),
                        field(&permission, "type"),
                        field(&permission, "role"),
                        field(&permission, "emailAddress"),
                        field(&permission, "displayName"),
                        field(&permission, "domain"),
                        permission.to_string(),
                        scan_id,
                    ],
                )?;
                summary.permissions_scanned += 1;
            }
        }

        if item.is_dir {
            summary.folders_scanned += 1;
        } else {
            summary.files_scanned += 1;
            summary.bytes_discovered = summary
                .bytes_discovered
                .saturating_add(size.unwrap_or(0) as u64);
            let file_size = size.unwrap_or(0) as u64;
            let mut ancestor = item.path.rsplit_once('/').map(|(parent, _)| parent);
            while let Some(folder_path) = ancestor {
                let total = folder_sizes.entry(folder_path).or_default();
                *total = total.saturating_add(file_size);
                ancestor = folder_path.rsplit_once('/').map(|(parent, _)| parent);
            }
        }
    }

    transaction.execute(
        "UPDATE drive_items
         SET cumulative_size_bytes = 0
         WHERE remote_name = ?1 AND is_directory = 1 AND last_seen_scan_id = ?2",
        params![remote, scan_id],
    )?;
    for (folder_path, size) in folder_sizes {
        transaction.execute(
            "UPDATE drive_items
             SET cumulative_size_bytes = ?3
             WHERE remote_name = ?1 AND relative_path = ?2 AND is_directory = 1",
            params![remote, folder_path, size as i64],
        )?;
    }
    if authoritative {
        summary.deleted_items = transaction.execute(
            "UPDATE drive_items
             SET is_deleted = 1,
                 deleted_at = COALESCE(deleted_at, CURRENT_TIMESTAMP)
             WHERE remote_name = ?1
               AND last_seen_scan_id <> ?2
               AND is_deleted = 0",
            params![remote, scan_id],
        )? as u64;
    }

    transaction.execute(
        "UPDATE scan_runs SET
            status = 'complete', completed_at = CURRENT_TIMESTAMP, error_message = NULL,
            files_scanned = ?2, folders_scanned = ?3, permissions_scanned = ?4,
            bytes_discovered = ?5, deleted_items = ?6
         WHERE id = ?1",
        params![
            scan_id,
            summary.files_scanned as i64,
            summary.folders_scanned as i64,
            summary.permissions_scanned as i64,
            summary.bytes_discovered as i64,
            summary.deleted_items as i64,
        ],
    )?;
    summary.completed_at = transaction.query_row(
        "SELECT completed_at FROM scan_runs WHERE id = ?1",
        [scan_id],
        |row| row.get(0),
    )?;

    transaction.commit()?;
    Ok(summary)
}

pub fn latest_summary(database: &Database) -> Result<Option<InventorySummary>, DatabaseError> {
    latest_summary_for(database, "my-drive")
}

pub fn latest_summary_for(
    database: &Database,
    scan_type: &str,
) -> Result<Option<InventorySummary>, DatabaseError> {
    let connection = database.connect()?;
    connection
        .query_row(
            "SELECT completed_at, files_scanned, folders_scanned,
                permissions_scanned, bytes_discovered, deleted_items
         FROM scan_runs
         WHERE scan_type = ?1 AND status = 'complete'
         ORDER BY id DESC LIMIT 1",
            [scan_type],
            |row| {
                Ok(InventorySummary {
                    completed_at: row.get(0)?,
                    files_scanned: row.get::<_, i64>(1)? as u64,
                    folders_scanned: row.get::<_, i64>(2)? as u64,
                    permissions_scanned: row.get::<_, i64>(3)? as u64,
                    bytes_discovered: row.get::<_, i64>(4)? as u64,
                    deleted_items: row.get::<_, i64>(5)? as u64,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

pub fn scan_timing_estimate(
    database: &Database,
    scan_type: &str,
) -> Result<Option<ScanTimingEstimate>, DatabaseError> {
    let connection = database.connect()?;
    let active_started_at: Option<String> = connection
        .query_row(
            "SELECT started_at FROM scan_runs
             WHERE scan_type = ?1 AND status = 'running'
             ORDER BY id DESC LIMIT 1",
            [scan_type],
            |row| row.get(0),
        )
        .optional()?;
    let Some(active_started_at) = active_started_at else {
        return Ok(None);
    };

    let elapsed_seconds = connection.query_row(
        "SELECT MAX(0, CAST((julianday('now') - julianday(?1)) * 86400 AS INTEGER))",
        [&active_started_at],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let (average_seconds, sample_count) = connection.query_row(
        "SELECT COALESCE(AVG(duration_seconds), 0), COUNT(*) FROM (
             SELECT (julianday(completed_at) - julianday(started_at)) * 86400 AS duration_seconds
             FROM scan_runs
             WHERE scan_type = ?1 AND status = 'complete' AND completed_at IS NOT NULL
             ORDER BY id DESC LIMIT 5
         )",
        [scan_type],
        |row| {
            Ok((
                row.get::<_, f64>(0)?.round() as u64,
                row.get::<_, i64>(1)? as u64,
            ))
        },
    )?;

    Ok(
        (sample_count > 0 && average_seconds > 0).then_some(ScanTimingEstimate {
            elapsed_seconds,
            average_seconds,
        }),
    )
}

fn permissions(item: &DriveItem) -> Result<Vec<Value>, DatabaseError> {
    let Some(raw) = item.metadata.get("permissions") else {
        return Ok(Vec::new());
    };
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| format!("Invalid permissions metadata in the rclone response: {error}"))?;
    Ok(match value {
        Value::Array(values) => values,
        Value::Null => Vec::new(),
        value => vec![value],
    })
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value.get(name).and_then(Value::as_str)
}

fn permission_key(value: &Value) -> String {
    if let Some(id) = field(value, "id") {
        return format!("id:{id}");
    }
    ["type", "role", "emailAddress", "domain"]
        .iter()
        .filter_map(|name| field(value, name).map(|part| format!("{name}:{part}")))
        .collect::<Vec<_>>()
        .join("|")
        .pipe_nonempty()
        .unwrap_or_else(|| value.to_string())
}

trait Nonempty {
    fn pipe_nonempty(self) -> Option<Self>
    where
        Self: Sized;
}
impl Nonempty for String {
    fn pipe_nonempty(self) -> Option<Self> {
        if self.is_empty() { None } else { Some(self) }
    }
}

fn empty_as_none(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}
