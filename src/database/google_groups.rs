use super::{Database, DatabaseError};
use crate::google::groups::{Group, Member, Snapshot};
use rusqlite::{OptionalExtension, params};

#[derive(Default)]
pub struct Summary {
    pub groups: u64,
    pub members: u64,
    pub restricted: u64,
    pub completed_at: String,
}
pub fn summary(db: &Database, account: &str) -> Result<Summary, DatabaseError> {
    let c = db.connect()?;
    Ok(Summary {
        groups: c.query_row(
            "SELECT COUNT(*) FROM google_groups WHERE account=?1",
            [account],
            |r| r.get::<_, i64>(0).map(|n| n as u64),
        )?,
        members: c.query_row(
            "SELECT COUNT(*) FROM google_group_members WHERE account=?1",
            [account],
            |r| r.get::<_, i64>(0).map(|n| n as u64),
        )?,
        restricted: c.query_row(
            "SELECT COUNT(*) FROM google_groups WHERE account=?1 AND members_unavailable=1",
            [account],
            |r| r.get::<_, i64>(0).map(|n| n as u64),
        )?,
        completed_at: c
            .query_row(
                "SELECT completed_at FROM google_groups_sync WHERE account=?1",
                [account],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default(),
    })
}
pub fn synchronize(db: &Database, snapshot: &Snapshot) -> Result<(), DatabaseError> {
    if snapshot.account.is_empty() {
        return Err("Google Groups account is missing".into());
    }
    let mut c = db.connect()?;
    let tx = c.transaction()?;
    tx.execute(
        "DELETE FROM google_groups WHERE account=?1",
        [&snapshot.account],
    )?;
    for group in &snapshot.groups {
        if group.id.is_empty()
            || group.email.is_empty()
            || (group.members_unavailable && !group.members.is_empty())
        {
            return Err("Invalid Google Groups report".into());
        }
        let mut aliases = group.aliases.clone();
        aliases.extend(group.non_editable_aliases.clone());
        aliases.sort();
        aliases.dedup();
        tx.execute("INSERT INTO google_groups(account,group_id,email,name,description,direct_members_count,aliases_json,members_unavailable) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![snapshot.account,group.id,group.email,group.name,group.description,group.direct_members_count,serde_json::to_string(&aliases)?,group.members_unavailable])?;
        for member in &group.members {
            if member.id.is_empty() {
                return Err("Google Groups member ID is missing".into());
            }
            tx.execute("INSERT INTO google_group_members(account,group_id,member_id,email,role,kind,status) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![snapshot.account,group.id,member.id,member.email,member.role,member.kind,member.status])?;
        }
    }
    tx.execute("INSERT INTO google_groups_sync(account) VALUES(?1) ON CONFLICT(account) DO UPDATE SET completed_at=CURRENT_TIMESTAMP",[&snapshot.account])?;
    tx.commit()?;
    Ok(())
}
pub fn list(db: &Database, account: &str) -> Result<Vec<Group>, DatabaseError> {
    let c = db.connect()?;
    let mut statement=c.prepare("SELECT group_id,email,name,description,direct_members_count,aliases_json,members_unavailable FROM google_groups WHERE account=?1 ORDER BY name COLLATE NOCASE,group_id")?;
    let mut groups = statement
        .query_map([account], |r| {
            Ok(Group {
                id: r.get(0)?,
                email: r.get(1)?,
                name: r.get(2)?,
                description: r.get(3)?,
                direct_members_count: r.get(4)?,
                aliases: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                members_unavailable: r.get(6)?,
                ..Default::default()
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut members=c.prepare("SELECT member_id,email,role,kind,status FROM google_group_members WHERE account=?1 AND group_id=?2 ORDER BY role,email COLLATE NOCASE,member_id")?;
    for group in &mut groups {
        group.members = members
            .query_map(params![account, group.id], |r| {
                Ok(Member {
                    id: r.get(0)?,
                    email: r.get(1)?,
                    role: r.get(2)?,
                    kind: r.get(3)?,
                    status: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_are_atomic_isolated_and_distinguish_restricted_lists() {
        let root = std::env::temp_dir().join(format!(
            "boreal-google-groups-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let runtime = crate::bootstrap::Runtime {
            boreal_home: root.clone(),
            boreal: serde_json::json!({}),
            directories: std::collections::BTreeMap::from([("SQLITE".into(), root.join("sqlite"))]),
        };
        let db = Database::initialize(&runtime).unwrap();
        let mut snapshot = Snapshot {
            account: "me@example.test".into(),
            groups: vec![Group {
                id: "g1".into(),
                email: "team@example.test".into(),
                members: vec![Member {
                    id: "m1".into(),
                    email: "member@example.test".into(),
                    role: "OWNER".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        synchronize(&db, &snapshot).unwrap();
        let summary = summary(&db, &snapshot.account).unwrap();
        assert_eq!(summary.groups, 1);
        assert_eq!(summary.members, 1);
        assert!(!summary.completed_at.is_empty());
        assert!(list(&db, "someone-else@example.test").unwrap().is_empty());
        snapshot.groups.push(snapshot.groups[0].clone());
        assert!(synchronize(&db, &snapshot).is_err());
        assert_eq!(list(&db, &snapshot.account).unwrap()[0].members.len(), 1);
        snapshot.groups.pop();
        snapshot.groups[0].members.clear();
        snapshot.groups[0].members_unavailable = true;
        synchronize(&db, &snapshot).unwrap();
        let groups = list(&db, &snapshot.account).unwrap();
        assert!(groups[0].members_unavailable);
        assert!(groups[0].members.is_empty());
        snapshot.groups.clear();
        synchronize(&db, &snapshot).unwrap();
        assert!(list(&db, &snapshot.account).unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
