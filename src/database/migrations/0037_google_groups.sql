CREATE TABLE google_groups (
    account TEXT NOT NULL,
    group_id TEXT NOT NULL,
    email TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    direct_members_count TEXT NOT NULL,
    aliases_json TEXT NOT NULL,
    members_unavailable INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(account, group_id)
);
CREATE TABLE google_group_members (
    account TEXT NOT NULL,
    group_id TEXT NOT NULL,
    member_id TEXT NOT NULL,
    email TEXT NOT NULL,
    role TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    PRIMARY KEY(account, group_id, member_id),
    FOREIGN KEY(account, group_id) REFERENCES google_groups(account, group_id) ON DELETE CASCADE
);
CREATE TABLE google_groups_sync (
    account TEXT PRIMARY KEY,
    completed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
