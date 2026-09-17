use super::{Database, DatabaseError};
use crate::local_files::Item;
use rusqlite::{Transaction, params};
use std::collections::{BTreeMap, HashMap};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug)]
pub struct Snapshot {
    pub id: i64,
    pub completed_at: String,
    pub scope: String,
    pub item_count: usize,
}

pub fn record(
    tx: &Transaction<'_>,
    items: &[Item],
    scope: &str,
    cancellation: Option<&Arc<AtomicBool>>,
) -> Result<(), DatabaseError> {
    tx.execute(
        "INSERT INTO local_file_snapshots(scope,item_count) VALUES(?1,?2)",
        params![scope, items.len() as i64],
    )?;
    let id = tx.last_insert_rowid();
    let mut insert = tx.prepare("INSERT INTO local_file_snapshot_items(snapshot_id,root_path,relative_path,metadata) VALUES(?1,?2,?3,?4)")?;
    for item in items {
        if cancellation.is_some_and(|v| v.load(Ordering::Acquire)) {
            return Err("Local Files snapshot cancelled".into());
        }
        insert.execute(params![
            id,
            item.root_path,
            item.relative_path,
            serde_json::to_string(item)?
        ])?;
    }
    Ok(())
}

pub fn snapshots(db: &Database) -> Result<Vec<Snapshot>, DatabaseError> {
    let c = db.connect()?;
    let mut s = c.prepare(
        "SELECT id,completed_at,scope,item_count FROM local_file_snapshots ORDER BY id DESC",
    )?;
    Ok(s.query_map([], |r| {
        Ok(Snapshot {
            id: r.get(0)?,
            completed_at: r.get(1)?,
            scope: r.get(2)?,
            item_count: r.get::<_, i64>(3)? as usize,
        })
    })?
    .collect::<Result<_, _>>()?)
}

fn items(db: &Database, id: i64) -> Result<Vec<Item>, DatabaseError> {
    let c = db.connect()?;
    let mut s = c.prepare("SELECT metadata FROM local_file_snapshot_items WHERE snapshot_id=?1")?;
    let rows = s.query_map([id], |r| r.get::<_, String>(0))?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

#[derive(Debug)]
pub struct Change {
    pub kind: &'static str,
    pub before: Option<Item>,
    pub after: Option<Item>,
    pub detail: String,
}

pub fn compare(
    db: &Database,
    before: i64,
    after: i64,
    same_scope: bool,
) -> Result<Vec<Change>, DatabaseError> {
    Ok(diff(items(db, before)?, items(db, after)?, same_scope))
}

fn key(item: &Item) -> (String, String) {
    (item.root_path.clone(), item.relative_path.clone())
}

fn changed_fields(a: &Item, b: &Item) -> String {
    let mut fields = Vec::new();
    if a.is_directory != b.is_directory
        || a.is_symlink != b.is_symlink
        || a.extension != b.extension
    {
        fields.push("Type");
    }
    if a.size_bytes != b.size_bytes {
        fields.push("Indexed size");
    }
    if a.modified_unix != b.modified_unix
        || a.modified_nanos
            .zip(b.modified_nanos)
            .is_some_and(|(a, b)| a != b)
    {
        fields.push("Modified time");
    }
    if !a.checksum_sha256.is_empty()
        && !b.checksum_sha256.is_empty()
        && a.checksum_sha256 != b.checksum_sha256
    {
        fields.push("Content checksum");
    }
    if !a.file_identity.is_empty()
        && !b.file_identity.is_empty()
        && a.file_identity != b.file_identity
    {
        fields.push("File identity (replaced)");
    }
    if (a.owner_username.as_str(), a.owner_identifier.as_str())
        != (b.owner_username.as_str(), b.owner_identifier.as_str())
    {
        fields.push("Owner");
    }
    if (a.group_name.as_str(), a.group_identifier.as_str())
        != (b.group_name.as_str(), b.group_identifier.as_str())
    {
        fields.push("Group");
    }
    if a.symlink_target != b.symlink_target {
        fields.push("Link target");
    }
    fields.join(", ")
}

// Pair only identities unique in BOTH full snapshots. Hard links and duplicate
// content are deliberately left as separate additions/removals when ambiguous.
fn match_key(item: &Item, checksum: bool) -> String {
    if checksum {
        if item.is_directory || item.is_symlink {
            return String::new();
        }
        item.checksum_sha256.clone()
    } else {
        item.file_identity.clone()
    }
}

fn counts(items: &[Item], checksum: bool) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for item in items {
        *counts.entry(match_key(item, checksum)).or_default() += 1;
    }
    counts
}

pub fn diff(before: Vec<Item>, after: Vec<Item>, same_scope: bool) -> Vec<Change> {
    let identity_counts = (counts(&before, false), counts(&after, false));
    let hash_counts = (counts(&before, true), counts(&after, true));
    let mut old: BTreeMap<_, _> = before.into_iter().map(|i| (key(&i), i)).collect();
    let mut new: BTreeMap<_, _> = after.into_iter().map(|i| (key(&i), i)).collect();
    let mut changes = Vec::new();
    for (checksum, (old_counts, new_counts)) in [(false, identity_counts), (true, hash_counts)] {
        if checksum {
            let shared: Vec<_> = old
                .keys()
                .filter(|k| new.contains_key(*k))
                .cloned()
                .collect();
            for key in shared {
                let a = old.remove(&key).unwrap();
                let b = new.remove(&key).unwrap();
                let detail = changed_fields(&a, &b);
                if !detail.is_empty() {
                    changes.push(Change {
                        kind: "Changed",
                        before: Some(a),
                        after: Some(b),
                        detail,
                    });
                }
            }
        }
        let destinations: HashMap<_, _> = new
            .iter()
            .filter_map(|(k, item)| {
                let identity = match_key(item, checksum);
                (!identity.is_empty()
                    && old_counts.get(&identity) == Some(&1)
                    && new_counts.get(&identity) == Some(&1))
                .then(|| (identity, k.clone()))
            })
            .collect();
        let pairs: Vec<_> = old
            .iter()
            .filter_map(|(k, item)| {
                let identity = match_key(item, checksum);
                let dest = destinations.get(&identity)?;
                if k == dest {
                    return None;
                }
                let b = &new[dest];
                if item.is_directory != b.is_directory || item.is_symlink != b.is_symlink {
                    return None;
                }
                Some((k.clone(), dest.clone()))
            })
            .collect();
        for (source, dest) in pairs {
            let a = old.remove(&source).unwrap();
            let b = new.remove(&dest).unwrap();
            let fields = changed_fields(&a, &b);
            let evidence = if checksum {
                "Matching unique checksum; move inferred"
            } else {
                "Matching unique filesystem identity; move inferred"
            };
            let detail = if fields.is_empty() {
                evidence.to_string()
            } else {
                format!("{evidence}; {fields}")
            };
            changes.push(Change {
                kind: "Moved",
                before: Some(a),
                after: Some(b),
                detail,
            });
        }
    }
    for (_, a) in old {
        changes.push(Change {
            kind: if same_scope {
                "Deleted"
            } else {
                "Removed from index"
            },
            before: Some(a),
            after: None,
            detail: String::new(),
        });
    }
    for (_, b) in new {
        changes.push(Change {
            kind: "Added",
            before: None,
            after: Some(b),
            detail: String::new(),
        });
    }
    changes.sort_by_key(|c| key(c.after.as_ref().or(c.before.as_ref()).unwrap()));
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(path: &str, identity: &str) -> Item {
        Item {
            root_path: "/root".into(),
            relative_path: path.into(),
            name: path.into(),
            file_identity: identity.into(),
            modified_nanos: Some(0),
            ..Item::default()
        }
    }
    #[test]
    fn baseline_precision_enrichment_is_not_a_file_change() {
        let mut a = item("a", "");
        a.modified_nanos = None;
        let mut b = a.clone();
        b.modified_nanos = Some(123);
        b.file_identity = "device:inode".into();
        assert!(diff(vec![a], vec![b], false).is_empty());
    }

    #[test]
    fn recognizes_moves_when_paths_are_swapped() {
        let before = vec![item("a", "1"), item("b", "2")];
        let after = vec![item("b", "1"), item("a", "2")];
        let changes = diff(before, after, true);
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.kind == "Moved"));
    }
    #[test]
    fn detects_changes_moves_and_deletions_without_confusing_hard_links() {
        let before = vec![
            item("changed", "1"),
            item("old", "2"),
            item("deleted", "3"),
            item("hard-a", "4"),
            item("hard-b", "4"),
        ];
        let mut changed = item("changed", "1");
        changed.modified_nanos = Some(1);
        let after = vec![
            changed,
            item("new", "2"),
            item("added", "5"),
            item("hard-c", "4"),
            item("hard-b", "4"),
        ];
        let changes = diff(before, after, true);
        assert_eq!(changes.iter().filter(|c| c.kind == "Moved").count(), 1);
        assert_eq!(changes.iter().filter(|c| c.kind == "Deleted").count(), 2);
        assert_eq!(changes.iter().filter(|c| c.kind == "Added").count(), 2);
        assert!(
            changes
                .iter()
                .any(|c| c.kind == "Changed" && c.detail == "Modified time")
        );
    }
    #[test]
    fn checksum_availability_is_not_a_change_and_scope_removal_is_not_deletion() {
        let a = item("a", "");
        let mut b = a.clone();
        b.checksum_sha256 = "hash".into();
        assert!(diff(vec![a.clone()], vec![b], true).is_empty());
        assert_eq!(diff(vec![a], vec![], false)[0].kind, "Removed from index");
    }
    #[test]
    fn detects_unique_checksum_moves_but_not_ambiguous_copies() {
        let mut a = item("a", "");
        a.checksum_sha256 = "hash".into();
        let mut b = a.clone();
        b.relative_path = "b".into();
        assert_eq!(
            diff(vec![a.clone()], vec![b.clone()], true)[0].kind,
            "Moved"
        );
        assert_eq!(diff(vec![a.clone()], vec![a, b], true)[0].kind, "Added");
    }
}
