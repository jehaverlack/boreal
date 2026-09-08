use rusqlite::{OptionalExtension, params};

use super::{Database, DatabaseError};

#[derive(Debug, Clone, Default)]
pub struct ImportSummary {
    pub rows_seen: u64,
    pub rows_created: u64,
    pub rows_updated: u64,
    pub rows_rejected: u64,
}

#[derive(Debug, Clone, Default)]
pub struct DirectorySummary {
    pub principals: u64,
    pub organizations: u64,
    pub groups: u64,
    pub former_or_departing: u64,
    pub sources: u64,
}

#[derive(Debug, Clone)]
pub struct PrincipalRow {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub primary_email: String,
    pub principal_type: String,
    pub status: String,
    pub departure_date: String,
    pub organizations: String,
    pub members: u64,
    pub owned_items: u64,
    pub permitted_items: u64,
    pub tags: Vec<IdentityTag>,
    pub notes: String,
}

#[derive(Debug, Clone)]
pub struct IdentityTag {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub color: String,
}

#[derive(Debug, Clone)]
pub struct PersonChoice {
    pub id: i64,
    pub display_name: String,
    pub primary_email: String,
    pub username: String,
    pub search_text: String,
}

#[derive(Debug, Clone)]
pub struct OrganizationRow {
    pub name: String,
    pub members: u64,
}

#[derive(Debug, Clone)]
pub struct RemoteAccountRow {
    pub remote_name: String,
    pub account_email: String,
    pub display_name: String,
    pub last_verified_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct LinkedSheetStatus {
    pub configured: bool,
    pub last_attempt_at: String,
    pub last_success_at: String,
    pub last_error: String,
}

pub fn linked_sheet_status(database: &Database) -> Result<LinkedSheetStatus, DatabaseError> {
    let connection = database.connect()?;
    connection
        .query_row(
            "SELECT 1, COALESCE(last_attempt_at, ''), COALESCE(last_success_at, ''),
                    COALESCE(last_error, '')
             FROM directory_sources WHERE name = 'Linked Google Sheet directory'",
            [],
            |row| {
                Ok(LinkedSheetStatus {
                    configured: row.get::<_, i64>(0)? != 0,
                    last_attempt_at: row.get(1)?,
                    last_success_at: row.get(2)?,
                    last_error: row.get(3)?,
                })
            },
        )
        .optional()
        .map(|value| value.unwrap_or_default())
        .map_err(Into::into)
}

#[derive(Debug, Clone)]
pub struct PrincipalAssociationRow {
    pub remote_name: String,
    pub item_id: String,
    pub name: String,
    pub relative_path: String,
    pub relationship: String,
    pub role: String,
    pub owner_email: String,
    pub is_deleted: bool,
}

pub fn summary(database: &Database) -> Result<DirectorySummary, DatabaseError> {
    let connection = database.connect()?;
    connection
        .query_row(
            "SELECT
            (SELECT COUNT(*) FROM principals),
            (SELECT COUNT(*) FROM organizations),
            (SELECT COUNT(*) FROM principals
             WHERE lower(trim(principal_type)) IN ('group', 'google group', 'google_group')),
            (SELECT COUNT(*) FROM principals
             WHERE lower(trim(principal_type)) IN ('person', 'user')
               AND status IN ('former', 'departing')),
            (SELECT COUNT(*) FROM directory_sources WHERE enabled = 1)",
            [],
            |row| {
                Ok(DirectorySummary {
                    principals: row.get::<_, i64>(0)? as u64,
                    organizations: row.get::<_, i64>(1)? as u64,
                    groups: row.get::<_, i64>(2)? as u64,
                    former_or_departing: row.get::<_, i64>(3)? as u64,
                    sources: row.get::<_, i64>(4)? as u64,
                })
            },
        )
        .map_err(Into::into)
}

pub fn list_principals(database: &Database) -> Result<Vec<PrincipalRow>, DatabaseError> {
    list_principals_filtered(database, "", "", "", "", "", "", "")
}

pub fn list_principals_filtered(
    database: &Database,
    name_filter: &str,
    email_filter: &str,
    type_filter: &str,
    status_filter: &str,
    departure_filter: &str,
    organization_filter: &str,
    tag_filter: &str,
) -> Result<Vec<PrincipalRow>, DatabaseError> {
    let connection = database.connect()?;
    let (departure_comparison, departure_value) = parse_date_filter(departure_filter)?;
    let (exclude_tag, tag_slug) = match tag_filter.trim().strip_prefix('!') {
        Some(slug) => (true, slug.trim()),
        None => (false, tag_filter.trim()),
    };
    let mut statement = connection.prepare(
        "WITH identity_emails AS (
            SELECT id AS principal_id, lower(primary_email) AS email
            FROM principals WHERE COALESCE(primary_email, '') <> ''
            UNION
            SELECT principal_id, lower(email) FROM principal_emails
         ),
         owned AS (
            SELECT ie.principal_id, COUNT(*) AS item_count
            FROM identity_emails ie
            JOIN drive_items di ON lower(di.owner_email) = ie.email AND di.is_deleted = 0
            GROUP BY ie.principal_id
         ),
         permitted AS (
            SELECT ie.principal_id,
                   COUNT(DISTINCT dp.remote_name || char(31) || dp.item_id) AS item_count
            FROM identity_emails ie
            JOIN drive_permissions dp ON lower(dp.email_address) = ie.email
            JOIN drive_items di ON di.remote_name = dp.remote_name
                               AND di.item_id = dp.item_id AND di.is_deleted = 0
            GROUP BY ie.principal_id
         ),
         memberships AS (
            SELECT parent_principal_id AS principal_id, COUNT(*) AS member_count
            FROM principal_memberships WHERE status = 'active'
            GROUP BY parent_principal_id
         ),
         principal_organizations AS (
            SELECT om.principal_id, group_concat(o.name, ', ') AS names
            FROM organization_memberships om
            JOIN organizations o ON o.id = om.organization_id
            GROUP BY om.principal_id
         )
         SELECT p.id, COALESCE(p.username, ''), COALESCE(p.display_name, ''), COALESCE(p.primary_email, ''),
                p.principal_type, p.status, COALESCE(p.departure_date, ''),
                COALESCE(po.names, ''), COALESCE(m.member_count, 0),
                COALESCE(owned.item_count, 0), COALESCE(permitted.item_count, 0),
                COALESCE(p.notes, '')
         FROM principals p
         LEFT JOIN principal_organizations po ON po.principal_id = p.id
         LEFT JOIN memberships m ON m.principal_id = p.id
         LEFT JOIN owned ON owned.principal_id = p.id
         LEFT JOIN permitted ON permitted.principal_id = p.id
         WHERE (?1 = '' OR instr(lower(COALESCE(p.display_name, '')), lower(?1)) > 0)
           AND (?2 = '' OR instr(lower(COALESCE(p.primary_email, '')), lower(?2)) > 0
                OR instr(lower(COALESCE(p.username, '')), lower(?2)) > 0
                OR EXISTS (
                    SELECT 1 FROM principal_emails pe
                    WHERE pe.principal_id = p.id
                      AND instr(lower(pe.email), lower(?2)) > 0
                ))
           AND (?3 = '' OR instr(lower(p.principal_type), lower(?3)) > 0)
           AND (?4 = '' OR instr(lower(p.status), lower(?4)) > 0)
           AND (?5 = 0 OR
                (?5 = 1 AND COALESCE(p.departure_date, '') > ?6) OR
                (?5 = 2 AND COALESCE(p.departure_date, '') >= ?6) OR
                (?5 = 3 AND COALESCE(p.departure_date, '') < ?6) OR
                (?5 = 4 AND COALESCE(p.departure_date, '') <= ?6) OR
                (?5 = 5 AND COALESCE(p.departure_date, '') = ?6))
           AND (?7 = '' OR instr(lower(COALESCE(po.names, '')), lower(?7)) > 0)
           AND (?8 = '' OR
                (?8 = '__untagged__' AND
                    ((?9 = 0 AND NOT EXISTS (
                        SELECT 1 FROM principal_tags untagged_pt
                        WHERE untagged_pt.principal_id = p.id
                    )) OR
                    (?9 = 1 AND EXISTS (
                        SELECT 1 FROM principal_tags untagged_pt
                        WHERE untagged_pt.principal_id = p.id
                    )))) OR
                (?8 <> '__untagged__' AND ?9 = 0 AND EXISTS (
                    SELECT 1
                    FROM principal_tags filter_pt
                    JOIN tags filter_tag ON filter_tag.id = filter_pt.tag_id
                    WHERE filter_pt.principal_id = p.id AND filter_tag.slug = ?8
                )) OR
                (?8 <> '__untagged__' AND ?9 = 1 AND NOT EXISTS (
                    SELECT 1
                    FROM principal_tags filter_pt
                    JOIN tags filter_tag ON filter_tag.id = filter_pt.tag_id
                    WHERE filter_pt.principal_id = p.id AND filter_tag.slug = ?8
                )))
         ORDER BY p.display_name COLLATE NOCASE, p.primary_email COLLATE NOCASE",
    )?;
    let rows = statement.query_map(
        params![
            name_filter.trim(),
            email_filter.trim(),
            type_filter.trim(),
            status_filter.trim(),
            departure_comparison,
            departure_value,
            organization_filter.trim(),
            tag_slug,
            exclude_tag,
        ],
        |row| {
            Ok(PrincipalRow {
                id: row.get(0)?,
                username: row.get(1)?,
                display_name: row.get(2)?,
                primary_email: row.get(3)?,
                principal_type: row.get(4)?,
                status: row.get(5)?,
                departure_date: row.get(6)?,
                organizations: row.get(7)?,
                members: row.get::<_, i64>(8)? as u64,
                owned_items: row.get::<_, i64>(9)? as u64,
                permitted_items: row.get::<_, i64>(10)? as u64,
                tags: Vec::new(),
                notes: row.get(11)?,
            })
        },
    )?;
    let mut principals = rows.collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for principal in &mut principals {
        let mut tag_statement = connection.prepare(
            "SELECT t.slug, t.name, t.description, t.color
             FROM principal_tags pt JOIN tags t ON t.id = pt.tag_id
             WHERE pt.principal_id = ?1 ORDER BY t.name COLLATE NOCASE",
        )?;
        principal.tags = tag_statement
            .query_map([principal.id], |row| {
                Ok(IdentityTag {
                    slug: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    color: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(principals)
}

pub fn list_principal_types(database: &Database) -> Result<Vec<String>, DatabaseError> {
    let connection = database.connect()?;
    let mut types = ["person", "group", "service_acct", "dept_acct", "other"]
        .into_iter()
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    let mut statement = connection.prepare(
        "SELECT DISTINCT principal_type FROM principals
         WHERE principal_type IS NOT NULL AND trim(principal_type) <> ''",
    )?;
    for principal_type in statement.query_map([], |row| row.get::<_, String>(0))? {
        types.insert(principal_type?);
    }
    Ok(types.into_iter().collect())
}

pub fn list_person_choices(database: &Database) -> Result<Vec<PersonChoice>, DatabaseError> {
    let connection = database.connect()?;
    let mut statement = connection.prepare(
        "SELECT p.id, COALESCE(p.display_name, ''), COALESCE(p.primary_email, ''),
                COALESCE(p.username, ''),
                COALESCE((
                    SELECT group_concat(pe.email, ' ')
                    FROM principal_emails pe
                    WHERE pe.principal_id = p.id
                ), '')
         FROM principals p
         WHERE lower(trim(p.principal_type)) NOT IN ('group', 'google group', 'google_group')
         ORDER BY p.display_name COLLATE NOCASE, p.primary_email COLLATE NOCASE,
                  p.username COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(PersonChoice {
            id: row.get(0)?,
            display_name: row.get(1)?,
            primary_email: row.get(2)?,
            username: row.get(3)?,
            search_text: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn associate_username(
    database: &Database,
    principal_id: i64,
    username: &str,
) -> Result<(), DatabaseError> {
    let username = username.trim();
    if username.is_empty() {
        return Err("A filesystem username is required".into());
    }
    let connection = database.connect()?;
    let updated = connection.execute(
        "UPDATE principals SET username = ?2, updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1
           AND NOT EXISTS (
               SELECT 1 FROM principals other
               WHERE other.id <> ?1 AND lower(other.username) = lower(?2)
           )",
        params![principal_id, username],
    )?;
    if updated == 0 {
        return Err("The person was not found or that username is already associated".into());
    }
    Ok(())
}

pub fn apply_principal_tag(
    database: &Database,
    principal_ids: &[i64],
    tag_slug: &str,
) -> Result<usize, DatabaseError> {
    if principal_ids.is_empty() {
        return Err("Select at least one directory identity".into());
    }
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let tag_id: i64 = transaction
        .query_row(
            "SELECT t.id FROM tags t JOIN tag_scopes s ON s.tag_id = t.id
             WHERE t.slug = ?1 AND s.scope = 'directory'",
            [tag_slug],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| format!("Tag '{tag_slug}' is not available for Directory"))?;
    let mut applied = 0;
    for principal_id in principal_ids {
        applied += transaction.execute(
            "INSERT OR IGNORE INTO principal_tags (principal_id, tag_id)
             SELECT id, ?2 FROM principals WHERE id = ?1",
            params![principal_id, tag_id],
        )?;
    }
    transaction.commit()?;
    Ok(applied)
}

pub fn remove_principal_tag(
    database: &Database,
    principal_ids: &[i64],
    tag_slug: &str,
) -> Result<usize, DatabaseError> {
    if principal_ids.is_empty() {
        return Err("Select at least one directory identity".into());
    }
    let connection = database.connect()?;
    let mut removed = 0;
    for principal_id in principal_ids {
        removed += connection.execute(
            "DELETE FROM principal_tags
             WHERE principal_id = ?1 AND tag_id = (SELECT id FROM tags WHERE slug = ?2)",
            params![principal_id, tag_slug],
        )?;
    }
    Ok(removed)
}

pub fn save_manual_principal(
    database: &Database,
    principal_id: Option<i64>,
    username: &str,
    email: &str,
    display_name: &str,
    principal_type: &str,
    status: &str,
    departure_date: &str,
    organization: &str,
    notes: &str,
) -> Result<i64, DatabaseError> {
    let username = username.trim();
    let email = email.trim().to_ascii_lowercase();
    if username.is_empty() && email.is_empty() {
        return Err("A username or primary email address is required".into());
    }
    if !email.is_empty() && !valid_email(&email) {
        return Err("Enter a valid primary email address or leave it blank".into());
    }
    if principal_type.trim().is_empty() {
        return Err("A directory identity type is required".into());
    }
    let principal_type = canonical_principal_type(principal_type);
    let status = if status.trim().is_empty() {
        "unknown".to_string()
    } else {
        normalize_value(status)
    };
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let email_owner: Option<i64> = if email.is_empty() {
        None
    } else {
        transaction
            .query_row(
                "SELECT principal_id FROM principal_emails WHERE lower(email) = lower(?1)",
                [&email],
                |row| row.get(0),
            )
            .optional()?
    };
    if email_owner.is_some() && email_owner != principal_id {
        return Err("That email address already belongs to another directory identity".into());
    }
    let username_owner: Option<i64> = if username.is_empty() {
        None
    } else {
        transaction
            .query_row(
                "SELECT id FROM principals WHERE lower(username) = lower(?1)",
                [username],
                |row| row.get(0),
            )
            .optional()?
    };
    if username_owner.is_some() && username_owner != principal_id {
        return Err("That username already belongs to another directory identity".into());
    }
    let id = if let Some(id) = principal_id {
        let updated = transaction.execute(
            "UPDATE principals SET principal_type = ?2, username = NULLIF(?3, ''),
                    primary_email = NULLIF(?4, ''), display_name = NULLIF(?5, ''), status = ?6,
                    departure_date = NULLIF(?7, ''), notes = NULLIF(?8, ''),
                    source = 'manual', updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![
                id,
                principal_type,
                username,
                email,
                display_name.trim(),
                status,
                departure_date.trim(),
                notes.trim()
            ],
        )?;
        if updated == 0 {
            return Err("Directory identity was not found".into());
        }
        id
    } else {
        transaction.execute(
            "INSERT INTO principals (
                principal_type, username, primary_email, display_name, status,
                departure_date, notes, source
             ) VALUES (?1, NULLIF(?2, ''), NULLIF(?3, ''), NULLIF(?4, ''), ?5, NULLIF(?6, ''), NULLIF(?7, ''), 'manual')",
            params![
                principal_type,
                username,
                email,
                display_name.trim(),
                status,
                departure_date.trim(),
                notes.trim()
            ],
        )?;
        transaction.last_insert_rowid()
    };
    transaction.execute(
        "DELETE FROM principal_emails WHERE principal_id = ?1 AND is_primary = 1",
        [id],
    )?;
    if !email.is_empty() {
        transaction.execute(
            "INSERT INTO principal_emails (principal_id, email, is_primary) VALUES (?1, ?2, 1)
             ON CONFLICT(email) DO UPDATE SET is_primary = 1",
            params![id, email],
        )?;
    }
    transaction.execute(
        "DELETE FROM organization_memberships WHERE principal_id = ?1",
        [id],
    )?;
    for organization in organization
        .split(',')
        .map(str::trim)
        .filter(|organization| !organization.is_empty())
    {
        transaction.execute(
            "INSERT INTO organizations (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            [organization],
        )?;
        let organization_id: i64 = transaction.query_row(
            "SELECT id FROM organizations WHERE name = ?1 COLLATE NOCASE",
            [organization],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO organization_memberships (organization_id, principal_id, status, source)
             VALUES (?1, ?2, ?3, 'manual')",
            params![organization_id, id, status],
        )?;
    }
    transaction.commit()?;
    Ok(id)
}

fn parse_date_filter(filter: &str) -> Result<(i64, String), DatabaseError> {
    let filter = filter.trim();
    if filter.is_empty() {
        return Ok((0, String::new()));
    }
    let (comparison, value) = if let Some(value) = filter.strip_prefix(">=") {
        (2, value)
    } else if let Some(value) = filter.strip_prefix("<=") {
        (4, value)
    } else if let Some(value) = filter.strip_prefix('>') {
        (1, value)
    } else if let Some(value) = filter.strip_prefix('<') {
        (3, value)
    } else if let Some(value) = filter.strip_prefix('=') {
        (5, value)
    } else {
        (5, filter)
    };
    let value = value.trim();
    let valid = value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| index == 4 || index == 7 || character.is_ascii_digit());
    if !valid {
        return Err(format!("Invalid departure-date filter '{filter}'. Try <2026-12-31.").into());
    }
    Ok((comparison, value.to_string()))
}

pub fn get_principal(
    database: &Database,
    principal_id: i64,
) -> Result<Option<PrincipalRow>, DatabaseError> {
    list_principals(database).map(|principals| {
        principals
            .into_iter()
            .find(|principal| principal.id == principal_id)
    })
}

pub fn list_principal_associations(
    database: &Database,
    principal_id: i64,
) -> Result<Vec<PrincipalAssociationRow>, DatabaseError> {
    let connection = database.connect()?;
    let mut statement = connection.prepare(
        "SELECT remote_name, item_id, name, relative_path, relationship, role,
                COALESCE(owner_email, ''), is_deleted
         FROM (
            SELECT di.remote_name, di.item_id, di.name, di.relative_path,
                   'Owner' AS relationship, 'owner' AS role, di.owner_email, di.is_deleted
            FROM drive_items di
            WHERE lower(COALESCE(di.owner_email, '')) IN (
                SELECT lower(primary_email) FROM principals WHERE id = ?1
                UNION SELECT lower(email) FROM principal_emails WHERE principal_id = ?1
            )
            UNION ALL
            SELECT di.remote_name, di.item_id, di.name, di.relative_path,
                   'Permission' AS relationship, COALESCE(dp.role, ''),
                   di.owner_email, di.is_deleted
            FROM drive_permissions dp
            JOIN drive_items di ON di.remote_name = dp.remote_name AND di.item_id = dp.item_id
            WHERE lower(COALESCE(dp.email_address, '')) IN (
                SELECT lower(primary_email) FROM principals WHERE id = ?1
                UNION SELECT lower(email) FROM principal_emails WHERE principal_id = ?1
            )
         )
         ORDER BY is_deleted, relationship, name COLLATE NOCASE, remote_name",
    )?;
    let rows = statement.query_map([principal_id], |row| {
        Ok(PrincipalAssociationRow {
            remote_name: row.get(0)?,
            item_id: row.get(1)?,
            name: row.get(2)?,
            relative_path: row.get(3)?,
            relationship: row.get(4)?,
            role: row.get(5)?,
            owner_email: row.get(6)?,
            is_deleted: row.get(7)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn list_organizations(database: &Database) -> Result<Vec<OrganizationRow>, DatabaseError> {
    let connection = database.connect()?;
    let mut statement = connection.prepare(
        "SELECT o.name, COUNT(om.principal_id)
         FROM organizations o
         LEFT JOIN organization_memberships om ON om.organization_id = o.id
         GROUP BY o.id
         ORDER BY o.name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(OrganizationRow {
            name: row.get(0)?,
            members: row.get::<_, i64>(1)? as u64,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn list_remote_accounts(database: &Database) -> Result<Vec<RemoteAccountRow>, DatabaseError> {
    let connection = database.connect()?;
    let mut statement = connection.prepare(
        "SELECT remote_name, COALESCE(account_email, ''), COALESCE(display_name, ''),
                last_verified_at
         FROM remote_accounts ORDER BY remote_name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(RemoteAccountRow {
            remote_name: row.get(0)?,
            account_email: row.get(1)?,
            display_name: row.get(2)?,
            last_verified_at: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn remote_account_email(
    database: &Database,
    remote_name: &str,
) -> Result<Option<String>, DatabaseError> {
    let connection = database.connect()?;
    connection
        .query_row(
            "SELECT account_email FROM remote_accounts
             WHERE remote_name = ?1 AND COALESCE(account_email, '') <> ''",
            [remote_name],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

pub fn save_remote_account(
    database: &Database,
    remote_name: &str,
    account_email: Option<&str>,
    display_name: Option<&str>,
    account_id: Option<&str>,
    raw_json: &str,
) -> Result<(), DatabaseError> {
    let connection = database.connect()?;
    connection.execute(
        "INSERT INTO remote_accounts (
            remote_name, account_email, display_name, account_id, raw_json
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(remote_name) DO UPDATE SET
            account_email = excluded.account_email,
            display_name = excluded.display_name,
            account_id = excluded.account_id,
            raw_json = excluded.raw_json,
            last_verified_at = CURRENT_TIMESTAMP",
        params![
            remote_name,
            account_email,
            display_name,
            account_id,
            raw_json
        ],
    )?;
    Ok(())
}

pub fn import_csv(
    database: &Database,
    filename: &str,
    data: &[u8],
) -> Result<ImportSummary, DatabaseError> {
    import_csv_source(
        database,
        &format!("CSV: {}", filename.trim()),
        "csv_upload",
        None,
        false,
        data,
    )
}

pub fn validate_csv(data: &[u8]) -> Result<(), DatabaseError> {
    let text = std::str::from_utf8(data).map_err(|_| "Directory CSV must use UTF-8 encoding")?;
    let rows = parse_csv(text)?;
    let headers = rows
        .first()
        .ok_or("Directory CSV is empty")?
        .iter()
        .map(|value| normalize_header(value))
        .collect::<Vec<_>>();
    if header_index(&headers, &["email", "primary_email", "email_address"]).is_none()
        && header_index(&headers, &["username", "user_name", "login"]).is_none()
    {
        Err("Directory CSV requires an email or username column".into())
    } else {
        Ok(())
    }
}

pub fn import_linked_sheet_csv(
    database: &Database,
    source_location: &str,
    data: &[u8],
) -> Result<ImportSummary, DatabaseError> {
    import_csv_source(
        database,
        "Linked Google Sheet directory",
        "google_sheet",
        Some(source_location),
        true,
        data,
    )
}

fn import_csv_source(
    database: &Database,
    source_name: &str,
    source_type: &str,
    source_location: Option<&str>,
    refresh_on_metadata_update: bool,
    data: &[u8],
) -> Result<ImportSummary, DatabaseError> {
    let text = std::str::from_utf8(data).map_err(|_| "Directory CSV must use UTF-8 encoding")?;
    let rows = parse_csv(text)?;
    if rows.is_empty() {
        return Err("Directory CSV is empty".into());
    }
    let headers: Vec<String> = rows[0]
        .iter()
        .map(|value| normalize_header(value))
        .collect();
    let email_index = header_index(&headers, &["email", "primary_email", "email_address"]);
    let username_index = header_index(&headers, &["username", "user_name", "login"]);
    if email_index.is_none() && username_index.is_none() {
        return Err("Directory CSV requires an email or username column".into());
    }
    let name_index = header_index(&headers, &["name", "display_name"]);
    let type_index = header_index(&headers, &["type", "category", "principal_type"]);
    let status_index = header_index(&headers, &["status", "employment_status"]);
    let departure_index = header_index(&headers, &["departure_date", "end_date"]);
    let organization_index = header_index(&headers, &["organization", "department"]);
    let notes_index = header_index(&headers, &["notes", "note"]);
    let tags_index = header_index(&headers, &["tags", "tag"]);

    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO directory_sources (
            name, source_type, source_location, enabled, refresh_on_metadata_update
         ) VALUES (?1, ?2, ?3, 1, ?4)
         ON CONFLICT(name) DO UPDATE SET
            source_type = excluded.source_type,
            source_location = excluded.source_location,
            enabled = 1,
            refresh_on_metadata_update = excluded.refresh_on_metadata_update,
            updated_at = CURRENT_TIMESTAMP",
        params![
            source_name,
            source_type,
            source_location,
            refresh_on_metadata_update
        ],
    )?;
    let source_id: i64 = transaction.query_row(
        "SELECT id FROM directory_sources WHERE name = ?1",
        [source_name],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO directory_import_runs (source_id) VALUES (?1)",
        [source_id],
    )?;
    let run_id = transaction.last_insert_rowid();
    let mut summary = ImportSummary::default();

    for row in rows.iter().skip(1) {
        if row.iter().all(|value| value.trim().is_empty()) {
            continue;
        }
        summary.rows_seen += 1;
        let email = email_index
            .map(|index| cell(row, index).trim().to_ascii_lowercase())
            .unwrap_or_default();
        let username = username_index
            .map(|index| cell(row, index).trim())
            .unwrap_or("");
        if (email.is_empty() && username.is_empty()) || (!email.is_empty() && !valid_email(&email))
        {
            summary.rows_rejected += 1;
            continue;
        }
        let display_name = name_index
            .map(|index| cell(row, index).trim())
            .unwrap_or("");
        let principal_type = type_index
            .map(|index| cell(row, index).trim())
            .filter(|value| !value.is_empty())
            .map(canonical_principal_type);
        let type_was_supplied = principal_type.is_some();
        let status = status_index
            .map(|index| cell(row, index).trim())
            .filter(|value| !value.is_empty())
            .map(normalize_value)
            .unwrap_or_else(|| "unknown".to_string());
        let departure = departure_index
            .map(|index| cell(row, index).trim())
            .filter(|value| !value.is_empty());
        let notes = notes_index
            .map(|index| cell(row, index).trim())
            .filter(|value| !value.is_empty());
        let existing_id: Option<i64> = transaction
            .query_row(
                "SELECT id FROM principals
                 WHERE (?1 <> '' AND lower(primary_email) = lower(?1))
                    OR (?2 <> '' AND lower(username) = lower(?2))
                 LIMIT 1",
                params![email, username],
                |row| row.get(0),
            )
            .optional()?;
        let principal_id = if let Some(id) = existing_id {
            transaction.execute(
                "UPDATE principals SET
                    username = COALESCE(NULLIF(?2, ''), username),
                    primary_email = COALESCE(NULLIF(?3, ''), primary_email),
                    principal_type = CASE WHEN ?4 THEN COALESCE(?5, principal_type) ELSE principal_type END,
                    display_name = COALESCE(NULLIF(?6, ''), display_name),
                    status = ?7, departure_date = ?8,
                    notes = COALESCE(?9, notes), source = ?10,
                    updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                params![id, username, email, type_was_supplied, principal_type, display_name, status, departure, notes, source_name],
            )?;
            id
        } else {
            transaction.execute(
                "INSERT INTO principals (
                    principal_type, username, primary_email, display_name, status,
                    departure_date, notes, source
                 ) VALUES (COALESCE(?1, 'person'), NULLIF(?2, ''), NULLIF(?3, ''),
                    NULLIF(?4, ''), ?5, ?6, ?7, ?8)",
                params![
                    principal_type,
                    username,
                    email,
                    display_name,
                    status,
                    departure,
                    notes,
                    source_name
                ],
            )?;
            transaction.last_insert_rowid()
        };
        if let Some(tags) = tags_index.map(|index| cell(row, index)) {
            for tag in tags.split(';').map(str::trim).filter(|tag| !tag.is_empty()) {
                let tag_slug = tag_reference_slug(tag);
                transaction.execute(
                    "INSERT OR IGNORE INTO principal_tags (principal_id, tag_id)
                     SELECT ?1, t.id
                     FROM tags t
                     JOIN tag_scopes s ON s.tag_id = t.id
                     WHERE s.scope = 'directory'
                       AND (lower(t.name) = lower(?2) OR lower(t.slug) = lower(?3))",
                    params![principal_id, tag, tag_slug],
                )?;
            }
        }
        if !email.is_empty() {
            transaction.execute(
                "INSERT OR IGNORE INTO principal_emails (principal_id, email, is_primary)
                 VALUES (?1, ?2, 1)",
                params![principal_id, email],
            )?;
        }
        // The imported row is authoritative for this identity's organization.
        // Clear prior memberships even when the CSV organization cell is blank.
        transaction.execute(
            "DELETE FROM organization_memberships WHERE principal_id = ?1",
            [principal_id],
        )?;
        if let Some(organization_name) = organization_index
            .map(|index| cell(row, index).trim())
            .filter(|value| !value.is_empty())
        {
            transaction.execute(
                "INSERT INTO organizations (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
                [organization_name],
            )?;
            let organization_id: i64 = transaction.query_row(
                "SELECT id FROM organizations WHERE name = ?1 COLLATE NOCASE",
                [organization_name],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT INTO organization_memberships (
                    organization_id, principal_id, status, source
                 ) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(organization_id, principal_id) DO UPDATE SET
                    status = excluded.status, source = excluded.source",
                params![organization_id, principal_id, status, source_name],
            )?;
        }
        if existing_id.is_some() {
            summary.rows_updated += 1;
        } else {
            summary.rows_created += 1;
        }
    }
    transaction.execute(
        "UPDATE directory_import_runs SET
            status = 'complete', completed_at = CURRENT_TIMESTAMP,
            rows_seen = ?2, rows_created = ?3, rows_updated = ?4, rows_rejected = ?5
         WHERE id = ?1",
        params![
            run_id,
            summary.rows_seen as i64,
            summary.rows_created as i64,
            summary.rows_updated as i64,
            summary.rows_rejected as i64,
        ],
    )?;
    transaction.execute(
        "UPDATE directory_sources SET
            last_attempt_at = CURRENT_TIMESTAMP, last_success_at = CURRENT_TIMESTAMP,
            last_error = NULL, updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
        [source_id],
    )?;
    transaction.commit()?;
    Ok(summary)
}

pub fn record_linked_sheet_failure(
    database: &Database,
    source_location: &str,
    error: &str,
) -> Result<(), DatabaseError> {
    let connection = database.connect()?;
    connection.execute(
        "INSERT INTO directory_sources (
            name, source_type, source_location, enabled, refresh_on_metadata_update,
            last_attempt_at, last_error
         ) VALUES ('Linked Google Sheet directory', 'google_sheet', ?1, 1, 1,
                   CURRENT_TIMESTAMP, ?2)
         ON CONFLICT(name) DO UPDATE SET
            source_location = excluded.source_location,
            enabled = 1,
            refresh_on_metadata_update = 1,
            last_attempt_at = CURRENT_TIMESTAMP,
            last_error = excluded.last_error,
            updated_at = CURRENT_TIMESTAMP",
        params![source_location, error],
    )?;
    Ok(())
}

fn canonical_principal_type(value: &str) -> String {
    value.trim().to_string()
}

fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, DatabaseError> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut characters = text.chars().peekable();
    let mut quoted = false;
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                field.push('"');
                characters.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => row.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !quoted && characters.peek() == Some(&'\n') => {}
            character => field.push(character),
        }
    }
    if quoted {
        return Err("Directory CSV contains an unterminated quoted field".into());
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn normalize_header(value: &str) -> String {
    normalize_value(value.trim_start_matches('\u{feff}'))
}

fn normalize_value(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn tag_reference_slug(value: &str) -> String {
    normalize_value(value).replace('_', "-")
}

fn header_index(headers: &[String], names: &[&str]) -> Option<usize> {
    headers
        .iter()
        .position(|header| names.contains(&header.as_str()))
}

fn cell(row: &[String], index: usize) -> &str {
    row.get(index).map(String::as_str).unwrap_or("")
}

fn valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

#[cfg(test)]
mod tests {
    use super::{canonical_principal_type, parse_csv};

    #[test]
    fn parses_quoted_csv_fields_and_newlines() {
        let rows = parse_csv(
            "email,notes\r\na@example.edu,\"One, two\"\r\nb@example.edu,\"Line 1\nLine 2\"\n",
        )
        .expect("CSV should parse");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1][1], "One, two");
        assert_eq!(rows[2][1], "Line 1\nLine 2");
    }

    #[test]
    fn preserves_principal_type_spelling_and_capitalization() {
        assert_eq!(canonical_principal_type("Google Group"), "Google Group");
        assert_eq!(
            canonical_principal_type("service account"),
            "service account"
        );
        assert_eq!(
            canonical_principal_type("departmental_acct"),
            "departmental_acct"
        );
        assert_eq!(canonical_principal_type("Staff"), "Staff");
        assert_eq!(
            canonical_principal_type("Affiliate Researcher"),
            "Affiliate Researcher"
        );
    }
}
