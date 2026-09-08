use rusqlite::{OptionalExtension, params};

use crate::keeper::client::SharedFolder;

use super::{Database, DatabaseError, inventory::Tag};

#[derive(Debug, Clone)]
pub struct SharedFolderRow {
    pub folder_uid: String,
    pub name: String,
    pub folder_type: String,
    pub folder_path: String,
    pub is_accessible: bool,
    pub access: Vec<AccessRow>,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone)]
pub struct AccessRow {
    pub shared_to: String,
    pub permissions: String,
    pub target_kind: String,
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub shared_folders: u64,
    pub shared_with: u64,
    pub managed_folders: u64,
    pub completed_at: String,
}

pub fn summary(database: &Database) -> Result<Summary, DatabaseError> {
    let connection = database.connect()?;
    connection
        .query_row(
            "SELECT COUNT(*),
                    COALESCE((SELECT COUNT(DISTINCT shared_to) FROM keeper_shared_folder_access a
                              JOIN keeper_shared_folders af ON af.folder_uid=a.folder_uid
                              WHERE af.is_accessible=1),0),
                    COALESCE(SUM(CASE WHEN EXISTS(
                        SELECT 1 FROM keeper_shared_folder_access a
                        WHERE a.folder_uid=f.folder_uid AND a.permissions LIKE '%Manage%'
                    ) THEN 1 ELSE 0 END),0),
                    COALESCE((SELECT value FROM settings WHERE key='keeper.last_sync_at'),'')
             FROM keeper_shared_folders f WHERE is_accessible=1",
            [],
            |row| {
                Ok(Summary {
                    shared_folders: row.get::<_, i64>(0)? as u64,
                    shared_with: row.get::<_, i64>(1)? as u64,
                    managed_folders: row.get::<_, i64>(2)? as u64,
                    completed_at: row.get(3)?,
                })
            },
        )
        .map_err(Into::into)
}

pub fn synchronize(database: &Database, folders: &[SharedFolder]) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute("UPDATE keeper_shared_folders SET is_accessible=0", [])?;
    for folder in folders {
        transaction.execute(
            "INSERT INTO keeper_shared_folders(folder_uid,name,folder_type,folder_path,is_accessible,last_seen_at)
             VALUES(?1,?2,?3,?4,1,CURRENT_TIMESTAMP)
             ON CONFLICT(folder_uid) DO UPDATE SET name=excluded.name,folder_type=excluded.folder_type,
                folder_path=excluded.folder_path,is_accessible=1,last_seen_at=CURRENT_TIMESTAMP",
            params![folder.folder_uid, folder.name, folder.folder_type, folder.folder_path],
        )?;
        transaction.execute(
            "DELETE FROM keeper_shared_folder_access WHERE folder_uid=?1",
            [&folder.folder_uid],
        )?;
        for access in &folder.access {
            transaction.execute(
                "INSERT OR IGNORE INTO keeper_shared_folder_access(folder_uid,shared_to,permissions,target_kind)
                 VALUES(?1,?2,?3,?4)",
                params![folder.folder_uid, access.shared_to, access.permissions, access.target_kind],
            )?;
        }
    }
    super::settings::set_in_transaction(
        &transaction,
        "keeper.last_sync_at",
        &transaction.query_row("SELECT CURRENT_TIMESTAMP", [], |row| {
            row.get::<_, String>(0)
        })?,
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn list(
    database: &Database,
    name: &str,
    path: &str,
    shared_to: &str,
    permission: &str,
    tag: &str,
    include_inaccessible: bool,
    sort: &str,
    descending: bool,
) -> Result<Vec<SharedFolderRow>, DatabaseError> {
    let (exclude_tag, tag_slug) = tag
        .trim()
        .strip_prefix('!')
        .map_or((false, tag.trim()), |value| (true, value.trim()));
    let order = match sort {
        "path" => "folder_path",
        "type" => "folder_type",
        "shared" => {
            "(SELECT COUNT(*) FROM keeper_shared_folder_access a WHERE a.folder_uid=f.folder_uid)"
        }
        _ => "name COLLATE NOCASE",
    };
    let direction = if descending { "DESC" } else { "ASC" };
    let sql = format!(
        "SELECT folder_uid,name,folder_type,folder_path,is_accessible
         FROM keeper_shared_folders f
         WHERE (?1 OR is_accessible=1)
           AND (?2='' OR name LIKE '%'||?2||'%')
           AND (?3='' OR folder_path LIKE '%'||?3||'%')
           AND (?4='' OR EXISTS(SELECT 1 FROM keeper_shared_folder_access a WHERE a.folder_uid=f.folder_uid AND a.shared_to LIKE '%'||?4||'%'))
           AND (?5='' OR EXISTS(SELECT 1 FROM keeper_shared_folder_access a WHERE a.folder_uid=f.folder_uid AND a.permissions LIKE '%'||?5||'%'))
           AND (?6='' OR
                (?8=1 AND ((?7=0 AND NOT EXISTS(SELECT 1 FROM keeper_shared_folder_tags ft WHERE ft.folder_uid=f.folder_uid))
                         OR (?7=1 AND EXISTS(SELECT 1 FROM keeper_shared_folder_tags ft WHERE ft.folder_uid=f.folder_uid))))
                OR (?8=0 AND ((?7=0 AND EXISTS(SELECT 1 FROM keeper_shared_folder_tags ft JOIN tags t ON t.id=ft.tag_id WHERE ft.folder_uid=f.folder_uid AND t.slug=?6))
                           OR (?7=1 AND NOT EXISTS(SELECT 1 FROM keeper_shared_folder_tags ft JOIN tags t ON t.id=ft.tag_id WHERE ft.folder_uid=f.folder_uid AND t.slug=?6)))))
         ORDER BY {order} {direction}, folder_uid {direction}"
    );
    let connection = database.connect()?;
    let mut statement = connection.prepare(&sql)?;
    let mut folders = statement
        .query_map(
            params![
                include_inaccessible,
                name.trim(),
                path.trim(),
                shared_to.trim(),
                permission.trim(),
                tag_slug,
                exclude_tag,
                tag_slug == super::inventory::UNTAGGED_TAG_FILTER
            ],
            |row| {
                Ok(SharedFolderRow {
                    folder_uid: row.get(0)?,
                    name: row.get(1)?,
                    folder_type: row.get(2)?,
                    folder_path: row.get(3)?,
                    is_accessible: row.get(4)?,
                    access: Vec::new(),
                    tags: Vec::new(),
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut access_statement = connection.prepare(
        "SELECT shared_to,permissions,target_kind FROM keeper_shared_folder_access WHERE folder_uid=?1 ORDER BY shared_to COLLATE NOCASE",
    )?;
    let mut tag_statement = connection.prepare(
        "SELECT t.slug,t.name,t.description,t.color,0,0,0,0,0,1
         FROM keeper_shared_folder_tags ft JOIN tags t ON t.id=ft.tag_id
         WHERE ft.folder_uid=?1 ORDER BY t.name COLLATE NOCASE",
    )?;
    for folder in &mut folders {
        folder.access = access_statement
            .query_map([&folder.folder_uid], |row| {
                Ok(AccessRow {
                    shared_to: row.get(0)?,
                    permissions: row.get(1)?,
                    target_kind: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        folder.tags = tag_statement
            .query_map([&folder.folder_uid], |row| {
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
                    local_files: false,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(folders)
}

pub fn change_tags(
    database: &Database,
    folder_uids: &[String],
    slug: &str,
    remove: bool,
) -> Result<usize, DatabaseError> {
    if folder_uids.is_empty() {
        return Err("Select at least one Keeper shared folder".into());
    }
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let tag_id: i64 = transaction
        .query_row(
            "SELECT t.id FROM tags t JOIN tag_scopes s ON s.tag_id=t.id WHERE t.slug=?1 AND s.scope='keeper-shared-folders'",
            [slug],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| format!("Tag '{slug}' is not available for Keeper shared folders"))?;
    let mut changed = 0;
    for uid in folder_uids {
        changed += if remove {
            transaction.execute(
                "DELETE FROM keeper_shared_folder_tags WHERE folder_uid=?1 AND tag_id=?2",
                params![uid, tag_id],
            )?
        } else {
            transaction.execute("INSERT OR IGNORE INTO keeper_shared_folder_tags(folder_uid,tag_id) SELECT folder_uid,?2 FROM keeper_shared_folders WHERE folder_uid=?1", params![uid,tag_id])?
        };
    }
    transaction.commit()?;
    Ok(changed)
}
