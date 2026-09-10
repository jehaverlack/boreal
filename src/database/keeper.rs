use std::collections::{HashMap, HashSet};

use super::{Database, DatabaseError, inventory::Tag};
use crate::keeper::client::VaultSnapshot;
use rusqlite::{OptionalExtension, params};

#[derive(Debug, Clone)]
pub struct EntryRow {
    pub uid: String,
    pub is_folder: bool,
    pub name: String,
    pub item_type: String,
    pub folder_path: String,
    pub parent_uid: String,
    pub is_accessible: bool,
    pub modified: String,
    pub modified_ms: i64,
    pub attachment_count: u32,
    pub size_bytes: i64,
    pub access: Vec<AccessRow>,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone)]
pub struct AccessRow {
    pub known: bool,
    pub tags: Vec<Tag>,
    pub shared_to: String,
    pub permissions: String,
    pub target_kind: String,
}

#[derive(Debug, Clone)]
pub struct FolderLocation {
    pub uid: String,
    pub path: String,
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub shared_folders: u64,
    pub folders: u64,
    pub records: u64,
    pub shared_with: u64,
    pub managed_folders: u64,
    pub completed_at: String,
}

pub fn summary(database: &Database) -> Result<Summary, DatabaseError> {
    let connection = database.connect()?;
    Ok(connection.query_row(
        "SELECT
          (SELECT COUNT(*) FROM keeper_shared_folders WHERE is_accessible=1 AND folder_type IN ('Shared Folder','Nested Share Folder','Nested Shared Folder')),
          (SELECT COUNT(*) FROM keeper_shared_folders WHERE is_accessible=1 AND folder_uid<>''),
          (SELECT COUNT(*) FROM keeper_records WHERE is_accessible=1),
          (SELECT COUNT(DISTINCT a.shared_to) FROM keeper_shared_folder_access a JOIN keeper_shared_folders f USING(folder_uid) WHERE f.is_accessible=1),
          (SELECT COUNT(DISTINCT a.folder_uid) FROM keeper_shared_folder_access a JOIN keeper_shared_folders f USING(folder_uid) WHERE f.is_accessible=1 AND a.permissions LIKE '%Manage%'),
          COALESCE((SELECT value FROM settings WHERE key='keeper.last_sync_at'),'')",
        [], |r| Ok(Summary { shared_folders:r.get::<_,i64>(0)? as u64, folders:r.get::<_,i64>(1)? as u64, records:r.get::<_,i64>(2)? as u64, shared_with:r.get::<_,i64>(3)? as u64, managed_folders:r.get::<_,i64>(4)? as u64, completed_at:r.get(5)? }))?)
}

pub fn synchronize(database: &Database, snapshot: &VaultSnapshot) -> Result<(), DatabaseError> {
    crate::keeper::client::validate_snapshot(snapshot)?;
    let mut connection = database.connect()?;
    let tx = connection.transaction()?;
    tx.execute("UPDATE keeper_shared_folders SET is_accessible=0", [])?;
    tx.execute("UPDATE keeper_records SET is_accessible=0", [])?;
    for folder in &snapshot.folders {
        tx.execute(
            "INSERT INTO keeper_shared_folders(folder_uid,parent_uid,name,folder_type,folder_path,is_accessible,last_seen_at)
             VALUES(?1,?2,?3,?4,?5,1,CURRENT_TIMESTAMP)
             ON CONFLICT(folder_uid) DO UPDATE SET parent_uid=excluded.parent_uid,name=excluded.name,
             folder_type=excluded.folder_type,folder_path=excluded.folder_path,is_accessible=1,last_seen_at=CURRENT_TIMESTAMP",
            params![folder.folder_uid,folder.parent_uid,folder.name,folder.folder_type,folder.folder_path])?;
        tx.execute(
            "DELETE FROM keeper_shared_folder_access WHERE folder_uid=?1",
            [&folder.folder_uid],
        )?;
        for access in &folder.access {
            tx.execute("INSERT OR IGNORE INTO keeper_shared_folder_access(folder_uid,shared_to,permissions,target_kind) VALUES(?1,?2,?3,?4)",
                params![folder.folder_uid,access.shared_to,access.permissions,access.target_kind])?;
        }
    }
    for record in &snapshot.records {
        tx.execute("INSERT INTO keeper_records(record_uid,title,record_type,modified_ms,version,attachment_count,size_bytes,is_accessible,last_seen_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,1,CURRENT_TIMESTAMP)
            ON CONFLICT(record_uid) DO UPDATE SET title=excluded.title,record_type=excluded.record_type,modified_ms=excluded.modified_ms,
            version=excluded.version,attachment_count=excluded.attachment_count,size_bytes=excluded.size_bytes,is_accessible=1,last_seen_at=CURRENT_TIMESTAMP",
            params![record.record_uid,record.title,record.record_type,record.modified_ms,record.version,record.attachment_count,record.size_bytes])?;
        // Removed records retain their last known locations and tags for review.
        tx.execute(
            "DELETE FROM keeper_record_folders WHERE record_uid=?1",
            [&record.record_uid],
        )?;
    }
    for membership in &snapshot.memberships {
        tx.execute(
            "INSERT OR IGNORE INTO keeper_record_folders(record_uid,folder_uid) VALUES(?1,?2)",
            params![membership.record_uid, membership.folder_uid],
        )?;
    }
    super::settings::set_in_transaction(
        &tx,
        "keeper.last_sync_at",
        &tx.query_row("SELECT CURRENT_TIMESTAMP", [], |r| r.get::<_, String>(0))?,
    )?;
    tx.commit()?;
    Ok(())
}

pub fn folder_locations(database: &Database) -> Result<Vec<FolderLocation>, DatabaseError> {
    let connection = database.connect()?;
    let mut stmt = connection.prepare("SELECT folder_uid,folder_path FROM keeper_shared_folders WHERE is_accessible=1 ORDER BY folder_path COLLATE NOCASE")?;
    Ok(stmt
        .query_map([], |r| {
            Ok(FolderLocation {
                uid: r.get(0)?,
                path: r.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

#[derive(Default)]
pub struct ListOptions<'a> {
    pub folder: &'a str,
    pub all: bool,
    pub name: &'a str,
    pub path: &'a str,
    pub shared_to: &'a str,
    pub permission: &'a str,
    pub tag: &'a str,
    pub user_tag: &'a str,
    pub include_inaccessible: bool,
    pub sort: &'a str,
    pub descending: bool,
}

pub fn list(
    database: &Database,
    options: &ListOptions<'_>,
) -> Result<Vec<EntryRow>, DatabaseError> {
    let connection = database.connect()?;
    let mut stmt = connection.prepare("SELECT folder_uid,parent_uid,name,folder_type,folder_path,is_accessible FROM keeper_shared_folders")?;
    let folders = stmt
        .query_map([], |r| {
            Ok(EntryRow {
                uid: r.get(0)?,
                parent_uid: r.get(1)?,
                name: r.get(2)?,
                item_type: r.get(3)?,
                folder_path: r.get(4)?,
                is_accessible: r.get(5)?,
                is_folder: true,
                modified: String::new(),
                modified_ms: 0,
                attachment_count: 0,
                size_bytes: 0,
                access: vec![],
                tags: vec![],
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let folder_map: HashMap<_, _> = folders.iter().map(|f| (f.uid.clone(), f)).collect();
    let mut access_map: HashMap<String, Vec<AccessRow>> = HashMap::new();
    let mut stmt = connection.prepare("SELECT folder_uid,shared_to,permissions,target_kind FROM keeper_shared_folder_access ORDER BY shared_to COLLATE NOCASE")?;
    for result in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            AccessRow {
                known: false,
                tags: Vec::new(),
                shared_to: r.get(1)?,
                permissions: r.get(2)?,
                target_kind: r.get(3)?,
            },
        ))
    })? {
        let (uid, access) = result?;
        access_map.entry(uid).or_default().push(access);
    }
    // Resolve primary addresses and aliases, without treating team names as users.
    let mut known = HashSet::new();
    let mut user_tags: HashMap<String, Vec<Tag>> = HashMap::new();
    let directory_tags =
        super::inventory::list_tags_for_scope(database, super::inventory::TagScope::Directory)?;
    let directory_tags: HashMap<_, _> = directory_tags
        .into_iter()
        .map(|t| (t.slug.clone(), t))
        .collect();
    let mut statement = connection.prepare("WITH emails AS (
        SELECT principal_id, lower(trim(email)) AS email FROM principal_emails
        UNION SELECT id, lower(trim(primary_email)) FROM principals WHERE primary_email IS NOT NULL)
        SELECT e.email,t.slug FROM emails e LEFT JOIN principal_tags pt ON pt.principal_id=e.principal_id LEFT JOIN tags t ON t.id=pt.tag_id ORDER BY t.name COLLATE NOCASE")?;
    for result in statement.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })? {
        let (email, slug) = result?;
        known.insert(email.clone());
        if let Some(tag) = slug.and_then(|s| directory_tags.get(&s)) {
            let tags = user_tags.entry(email).or_default();
            if !tags.iter().any(|t| t.slug == tag.slug) {
                tags.push(tag.clone());
            }
        }
    }
    for access in access_map.values_mut().flatten() {
        if access.target_kind == "team" {
            continue;
        }
        let email = access
            .shared_to
            .strip_prefix("(Team User)")
            .unwrap_or(&access.shared_to)
            .trim()
            .to_lowercase();
        access.known = known.contains(&email);
        access.tags = user_tags.get(&email).cloned().unwrap_or_default();
    }
    // Show folder sharing context on records; this is not a claim about direct record grants.
    let inherited_access = |uid: &str| {
        let mut access = Vec::new();
        let mut current = uid.to_string();
        let mut seen = HashSet::new();
        while seen.insert(current.clone()) {
            if let Some(rows) = access_map.get(&current) {
                access.extend(rows.iter().cloned());
            }
            if current.is_empty() {
                break;
            }
            match folder_map.get(&current) {
                Some(folder)
                    if matches!(
                        folder.item_type.as_str(),
                        "Shared Folder" | "Nested Share Folder" | "Nested Shared Folder"
                    ) =>
                {
                    break;
                }
                Some(folder) => current = folder.parent_uid.clone(),
                None => break,
            }
        }
        access.sort_by(|a, b| (&a.shared_to, &a.permissions).cmp(&(&b.shared_to, &b.permissions)));
        access.dedup_by(|a, b| a.shared_to == b.shared_to && a.permissions == b.permissions);
        access
    };
    let tags = super::inventory::list_tags_for_scope(
        database,
        super::inventory::TagScope::KeeperSharedFolders,
    )?;
    let tag_map: HashMap<_, _> = tags.into_iter().map(|t| (t.slug.clone(), t)).collect();
    let mut entry_tags: HashMap<(bool, String), Vec<Tag>> = HashMap::new();
    let mut stmt = connection.prepare("SELECT 1,ft.folder_uid,t.slug FROM keeper_shared_folder_tags ft JOIN tags t ON t.id=ft.tag_id
        UNION ALL SELECT 0,rt.record_uid,t.slug FROM keeper_record_tags rt JOIN tags t ON t.id=rt.tag_id")?;
    for result in stmt.query_map([], |r| {
        Ok((
            r.get::<_, bool>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (folder, uid, slug) = result?;
        if let Some(tag) = tag_map.get(&slug) {
            entry_tags
                .entry((folder, uid))
                .or_default()
                .push(tag.clone());
        }
    }
    let mut entries = folders
        .iter()
        .filter(|f| !f.uid.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let mut stmt = connection.prepare("SELECT r.record_uid,r.title,r.record_type,r.modified_ms,r.attachment_count,r.size_bytes,
        r.is_accessible AND f.is_accessible,f.folder_path,f.folder_uid,
        CASE WHEN r.modified_ms>0 THEN COALESCE(strftime('%Y-%m-%d %H:%M',r.modified_ms/1000,'unixepoch'),'') ELSE '' END
        FROM keeper_records r JOIN keeper_record_folders rf ON rf.record_uid=r.record_uid
        JOIN keeper_shared_folders f ON f.folder_uid=rf.folder_uid")?;
    entries.extend(
        stmt.query_map([], |r| {
            Ok(EntryRow {
                uid: r.get(0)?,
                name: r.get(1)?,
                item_type: r.get(2)?,
                modified_ms: r.get(3)?,
                attachment_count: r.get(4)?,
                size_bytes: r.get(5)?,
                is_accessible: r.get(6)?,
                folder_path: r.get(7)?,
                parent_uid: r.get(8)?,
                modified: r.get(9)?,
                is_folder: false,
                access: vec![],
                tags: vec![],
            })
        })?
        .collect::<Result<Vec<_>, _>>()?,
    );
    for entry in &mut entries {
        entry.access = inherited_access(if entry.is_folder {
            &entry.uid
        } else {
            &entry.parent_uid
        });
        entry.tags = entry_tags
            .get(&(entry.is_folder, entry.uid.clone()))
            .cloned()
            .unwrap_or_default();
        entry.tags.sort_by_key(|tag| tag.name.to_lowercase());
    }
    let contains =
        |value: &str, filter: &str| value.to_lowercase().contains(&filter.trim().to_lowercase());
    let (excluded, tag) = options
        .tag
        .strip_prefix('!')
        .map_or((false, options.tag), |s| (true, s));
    let (exclude_user_tag, user_tag) = options
        .user_tag
        .strip_prefix('!')
        .map_or((false, options.user_tag), |s| (true, s));
    entries.retain(|e| {
        (user_tag.is_empty()
            || (e
                .access
                .iter()
                .any(|a| a.target_kind != "team" && a.tags.iter().any(|t| t.slug == user_tag))
                != exclude_user_tag))
            && (options.include_inaccessible || e.is_accessible)
            && (options.all || e.parent_uid == options.folder)
            && contains(&e.name, options.name)
            && contains(&e.folder_path, options.path)
            && (options.shared_to.trim().is_empty()
                || e.access
                    .iter()
                    .any(|a| contains(&a.shared_to, options.shared_to)))
            && (options.permission.trim().is_empty()
                || e.access
                    .iter()
                    .any(|a| contains(&a.permissions, options.permission)))
            && (tag.is_empty()
                || ((if tag == super::inventory::UNTAGGED_TAG_FILTER {
                    e.tags.is_empty()
                } else {
                    e.tags.iter().any(|t| t.slug == tag)
                }) != excluded))
    });
    entries.sort_by(|a, b| {
        let order = match options.sort {
            "path" => a
                .folder_path
                .to_lowercase()
                .cmp(&b.folder_path.to_lowercase()),
            "type" => a.item_type.to_lowercase().cmp(&b.item_type.to_lowercase()),
            "modified" => a.modified_ms.cmp(&b.modified_ms),
            "attachments" => a.attachment_count.cmp(&b.attachment_count),
            "size" => a.size_bytes.cmp(&b.size_bytes),
            "shared" => a
                .access
                .iter()
                .map(|r| r.shared_to.to_lowercase())
                .collect::<Vec<_>>()
                .cmp(
                    &b.access
                        .iter()
                        .map(|r| r.shared_to.to_lowercase())
                        .collect::<Vec<_>>(),
                ),
            "permission" => a
                .access
                .iter()
                .map(|r| r.permissions.to_lowercase())
                .collect::<Vec<_>>()
                .cmp(
                    &b.access
                        .iter()
                        .map(|r| r.permissions.to_lowercase())
                        .collect::<Vec<_>>(),
                ),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
        .then_with(|| (&a.uid, &a.parent_uid).cmp(&(&b.uid, &b.parent_uid)));
        if options.descending {
            order.reverse()
        } else {
            order
        }
    });
    Ok(entries)
}

pub fn change_tags(
    database: &Database,
    folder_uids: &[String],
    record_uids: &[String],
    slug: &str,
    remove: bool,
) -> Result<usize, DatabaseError> {
    if folder_uids.is_empty() && record_uids.is_empty() {
        return Err("Select at least one Keeper folder or record".into());
    }
    let mut connection = database.connect()?;
    let tx = connection.transaction()?;
    // Existing Keeper tag scopes apply to both folders and records; preserve old assignments.
    let tag_id: i64 = tx.query_row("SELECT t.id FROM tags t JOIN tag_scopes s ON s.tag_id=t.id WHERE t.slug=?1 AND s.scope='keeper-shared-folders'", [slug], |r| r.get(0)).optional()?.ok_or("Tag is not available for Keeper")?;
    let mut changed = 0;
    for (ids, table, column, source) in [
        (
            folder_uids,
            "keeper_shared_folder_tags",
            "folder_uid",
            "keeper_shared_folders",
        ),
        (
            record_uids,
            "keeper_record_tags",
            "record_uid",
            "keeper_records",
        ),
    ] {
        for uid in ids.iter().collect::<HashSet<_>>() {
            changed += if remove {
                tx.execute(
                    &format!("DELETE FROM {table} WHERE {column}=?1 AND tag_id=?2"),
                    params![uid, tag_id],
                )?
            } else {
                tx.execute(&format!("INSERT OR IGNORE INTO {table}({column},tag_id) SELECT {column},?2 FROM {source} WHERE {column}=?1"),params![uid,tag_id])?
            };
        }
    }
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::Runtime;
    use crate::keeper::client::{FolderAccess, Membership, Record, SharedFolder};

    struct TestDb {
        database: Database,
        root: std::path::PathBuf,
    }
    impl TestDb {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "boreal-keeper-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let runtime = Runtime {
                boreal_home: root.clone(),
                boreal: serde_json::json!({}),
                directories: std::collections::BTreeMap::from([(
                    "SQLITE".into(),
                    root.join("sqlite"),
                )]),
            };
            Self {
                database: Database::initialize(&runtime).unwrap(),
                root,
            }
        }
    }
    impl Drop for TestDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture() -> VaultSnapshot {
        let folder = |uid: &str, parent: &str, name: &str, kind: &str, path: &str| SharedFolder {
            folder_uid: uid.into(),
            parent_uid: parent.into(),
            name: name.into(),
            folder_type: kind.into(),
            folder_path: path.into(),
            access: vec![],
        };
        let mut shared = folder("shared", "", "Operations", "Shared Folder", "/Operations");
        shared.access.push(FolderAccess {
            shared_to: "person@example.test".into(),
            permissions: "Can Manage Users".into(),
            target_kind: "user".into(),
        });
        VaultSnapshot {
            schema_version: 1,
            folders: vec![
                folder("", "", "My Vault", "Vault", "/"),
                folder("personal", "", "Personal", "Folder", "/Personal"),
                shared,
                folder(
                    "nested",
                    "shared",
                    "Nested",
                    "Folder in Shared Folder",
                    "/Operations/Nested",
                ),
            ],
            records: vec![
                Record {
                    record_uid: "one".into(),
                    title: "Zebra".into(),
                    record_type: "login".into(),
                    modified_ms: 1700000000000,
                    version: 3,
                    attachment_count: 2,
                    size_bytes: 0,
                },
                Record {
                    record_uid: "two".into(),
                    title: "Alpha".into(),
                    record_type: "secureNote".into(),
                    modified_ms: 1700000001000,
                    version: 3,
                    attachment_count: 0,
                    size_bytes: 0,
                },
            ],
            memberships: vec![
                Membership {
                    record_uid: "one".into(),
                    folder_uid: "personal".into(),
                },
                Membership {
                    record_uid: "one".into(),
                    folder_uid: "nested".into(),
                },
                Membership {
                    record_uid: "two".into(),
                    folder_uid: "".into(),
                },
            ],
        }
    }

    #[test]
    fn keeper_user_tags_resolve_aliases_and_filter_permission_identities() {
        let db = TestDb::new();
        let person = super::super::directory::save_manual_principal(
            &db.database,
            None,
            "person",
            "primary@example.test",
            "Person",
            "person",
            "active",
            "",
            "",
            "",
        )
        .unwrap();
        db.database
            .connect()
            .unwrap()
            .execute(
                "INSERT INTO principal_emails(principal_id,email) VALUES(?1,?2)",
                params![person, "person@example.test"],
            )
            .unwrap();
        db.database.connect().unwrap().execute("INSERT OR IGNORE INTO tag_scopes(tag_id,scope) SELECT id,'directory' FROM tags WHERE slug='needs-review'", []).unwrap();
        super::super::directory::apply_principal_tag(&db.database, &[person], "needs-review")
            .unwrap();
        let mut snapshot = fixture();
        snapshot.folders[2].access[0].shared_to = "PERSON@EXAMPLE.TEST".into();
        snapshot.folders[2].access.push(FolderAccess {
            shared_to: "(Team User) primary@example.test".into(),
            permissions: "Read Only".into(),
            target_kind: "team-user".into(),
        });
        snapshot.folders[1].access.push(FolderAccess {
            shared_to: "person@example.test".into(),
            permissions: "Read Only".into(),
            target_kind: "team".into(),
        });
        synchronize(&db.database, &snapshot).unwrap();
        let matched = list(
            &db.database,
            &ListOptions {
                all: true,
                user_tag: "needs-review",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(matched.len(), 3);
        assert!(matched.iter().all(|entry| {
            entry
                .access
                .iter()
                .all(|access| access.known && access.tags[0].slug == "needs-review")
        }));
        let excluded = list(
            &db.database,
            &ListOptions {
                all: true,
                user_tag: "!needs-review",
                ..Default::default()
            },
        )
        .unwrap();
        assert!(excluded.iter().any(|entry| entry.uid == "personal"));
        assert!(
            excluded
                .iter()
                .all(|entry| !entry.access.iter().any(|a| !a.tags.is_empty()))
        );
        assert!(
            list(
                &db.database,
                &ListOptions {
                    all: true,
                    user_tag: "needs-review",
                    permission: "no-such-permission",
                    ..Default::default()
                }
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn keeper_vault_hierarchy_sharing_and_shortcuts() {
        let db = TestDb::new();
        synchronize(&db.database, &fixture()).unwrap();
        let root = list(&db.database, &ListOptions::default()).unwrap();
        assert_eq!(root.len(), 3);
        assert!(root.iter().any(|e| !e.is_folder && e.uid == "two"));
        let nested = list(
            &db.database,
            &ListOptions {
                folder: "nested",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].uid, "one");
        assert_eq!(nested[0].modified, "2023-11-14 22:13");
        assert_eq!(nested[0].access[0].shared_to, "person@example.test");
        let personal = list(
            &db.database,
            &ListOptions {
                folder: "personal",
                ..Default::default()
            },
        )
        .unwrap();
        assert!(personal[0].access.is_empty());
        let summary = summary(&db.database).unwrap();
        assert_eq!(
            (summary.folders, summary.shared_folders, summary.records),
            (3, 1, 2)
        );
        assert_eq!(folder_locations(&db.database).unwrap().len(), 4);
    }

    #[test]
    fn keeper_record_tags_follow_identity_and_survive_refresh() {
        let db = TestDb::new();
        let mut snapshot = fixture();
        synchronize(&db.database, &snapshot).unwrap();
        assert_eq!(
            change_tags(
                &db.database,
                &["personal".into()],
                &["one".into(), "one".into()],
                "needs-review",
                false
            )
            .unwrap(),
            2
        );
        let tagged = list(
            &db.database,
            &ListOptions {
                all: true,
                tag: "needs-review",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(tagged.len(), 3); // One folder, two memberships for the same record.
        assert_eq!(tagged.iter().filter(|e| !e.is_folder).count(), 2);
        snapshot.memberships.retain(|m| m.record_uid != "one");
        snapshot.memberships.push(Membership {
            folder_uid: "shared".into(),
            record_uid: "one".into(),
        });
        synchronize(&db.database, &snapshot).unwrap();
        let moved = list(
            &db.database,
            &ListOptions {
                folder: "shared",
                tag: "needs-review",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].uid, "one");
        snapshot.records.retain(|r| r.record_uid != "one");
        snapshot.memberships.retain(|m| m.record_uid != "one");
        synchronize(&db.database, &snapshot).unwrap();
        assert!(
            list(
                &db.database,
                &ListOptions {
                    all: true,
                    tag: "needs-review",
                    ..Default::default()
                }
            )
            .unwrap()
            .iter()
            .all(|e| e.is_folder)
        );
        let historical = list(
            &db.database,
            &ListOptions {
                all: true,
                tag: "needs-review",
                include_inaccessible: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(historical.iter().any(|e| !e.is_folder && !e.is_accessible));
        synchronize(&db.database, &fixture()).unwrap();
        assert_eq!(
            change_tags(&db.database, &[], &["one".into()], "needs-review", true).unwrap(),
            1
        );
        assert_eq!(
            list(
                &db.database,
                &ListOptions {
                    all: true,
                    tag: "needs-review",
                    ..Default::default()
                }
            )
            .unwrap()
            .len(),
            1
        );
    }

    #[test]
    fn keeper_filters_and_column_sorts() {
        let db = TestDb::new();
        synchronize(&db.database, &fixture()).unwrap();
        let all = list(
            &db.database,
            &ListOptions {
                all: true,
                sort: "name",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(all.first().unwrap().name, "Alpha");
        let desc = list(
            &db.database,
            &ListOptions {
                all: true,
                sort: "modified",
                descending: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(desc.first().unwrap().uid, "two");
        let attachments = list(
            &db.database,
            &ListOptions {
                all: true,
                sort: "attachments",
                descending: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(attachments.first().unwrap().attachment_count, 2);
        let filtered = list(
            &db.database,
            &ListOptions {
                all: true,
                name: "zEb",
                path: "nested",
                shared_to: "PERSON",
                permission: "manage",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        for sort in [
            "name",
            "path",
            "type",
            "modified",
            "attachments",
            "size",
            "shared",
            "permission",
        ] {
            let asc = list(
                &db.database,
                &ListOptions {
                    all: true,
                    sort,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut desc = list(
                &db.database,
                &ListOptions {
                    all: true,
                    sort,
                    descending: true,
                    ..Default::default()
                },
            )
            .unwrap();
            desc.reverse();
            assert_eq!(
                asc.iter()
                    .map(|e| (&e.uid, &e.parent_uid))
                    .collect::<Vec<_>>(),
                desc.iter()
                    .map(|e| (&e.uid, &e.parent_uid))
                    .collect::<Vec<_>>()
            );
        }
        change_tags(&db.database, &[], &["one".into()], "needs-review", false).unwrap();
        let untagged = list(
            &db.database,
            &ListOptions {
                all: true,
                tag: super::super::inventory::UNTAGGED_TAG_FILTER,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(untagged.iter().all(|e| e.uid != "one"));
        let excluded = list(
            &db.database,
            &ListOptions {
                all: true,
                tag: "!needs-review",
                ..Default::default()
            },
        )
        .unwrap();
        assert!(excluded.iter().all(|e| e.uid != "one"));
    }

    #[test]
    fn keeper_rejects_incomplete_refresh_without_changing_inventory() {
        let db = TestDb::new();
        synchronize(&db.database, &fixture()).unwrap();
        let mut bad = fixture();
        bad.records.clear();
        assert!(synchronize(&db.database, &bad).is_err());
        assert_eq!(summary(&db.database).unwrap().records, 2);
        let mut bad = fixture();
        bad.folders[1].parent_uid = "personal".into();
        assert!(synchronize(&db.database, &bad).is_err());
        assert_eq!(summary(&db.database).unwrap().folders, 3);
        let empty = VaultSnapshot {
            schema_version: 1,
            folders: vec![fixture().folders.remove(0)],
            records: vec![],
            memberships: vec![],
        };
        synchronize(&db.database, &empty).unwrap();
        assert!(
            list(
                &db.database,
                &ListOptions {
                    all: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            !list(
                &db.database,
                &ListOptions {
                    all: true,
                    include_inaccessible: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .is_empty()
        );
    }
}
