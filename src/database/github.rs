use rusqlite::{OptionalExtension, params};

use crate::github::client::Repository;

use super::{Database, DatabaseError, inventory::Tag};

#[derive(Debug, Clone)]
pub struct RepositoryRow {
    pub repository_id: i64,
    pub name: String,
    pub html_url: String,
    pub description: String,
    pub owner_login: String,
    pub owner_kind: String,
    pub visibility: String,
    pub archived: bool,
    pub fork: bool,
    pub language: String,
    pub size_kb: u64,
    pub effective_permission: String,
    pub pushed_at: String,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub repositories: u64,
    pub organizations: u64,
    pub private_repositories: u64,
    pub archived_repositories: u64,
    pub total_size_kb: u64,
    pub total_size_label: String,
    pub completed_at: String,
}

pub fn summary(database: &Database) -> Result<Summary, DatabaseError> {
    let connection = database.connect()?;
    let mut summary = connection
        .query_row(
            "SELECT COUNT(*),
                    COUNT(DISTINCT CASE WHEN owner_kind='Organization' THEN owner_id END),
                    COALESCE(SUM(CASE WHEN visibility='private' THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN archived=1 THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(size_kb),0),
                    COALESCE((SELECT value FROM settings WHERE key='github.last_sync_at'),'')
             FROM github_repositories WHERE is_accessible=1",
            [],
            |row| {
                Ok(Summary {
                    repositories: row.get::<_, i64>(0)? as u64,
                    organizations: row.get::<_, i64>(1)? as u64,
                    private_repositories: row.get::<_, i64>(2)? as u64,
                    archived_repositories: row.get::<_, i64>(3)? as u64,
                    total_size_kb: row.get::<_, i64>(4)? as u64,
                    total_size_label: String::new(),
                    completed_at: row.get(5)?,
                })
            },
        )
        .map_err(DatabaseError::from)?;
    summary.total_size_label = format_repository_size(summary.total_size_kb);
    Ok(summary)
}

fn format_repository_size(size_kb: u64) -> String {
    let bytes = size_kb.saturating_mul(1_000);
    const UNITS: [(&str, u64); 4] = [
        ("TB", 1_000_000_000_000),
        ("GB", 1_000_000_000),
        ("MB", 1_000_000),
        ("KB", 1_000),
    ];
    for (unit, divisor) in UNITS {
        if bytes >= divisor {
            return format!("{:.1} {unit}", bytes as f64 / divisor as f64);
        }
    }
    "0.0 KB".to_string()
}

pub fn synchronize(database: &Database, repositories: &[Repository]) -> Result<(), DatabaseError> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute("UPDATE github_repositories SET is_accessible = 0", [])?;
    transaction.execute("UPDATE github_organizations SET is_accessible = 0", [])?;
    for repository in repositories {
        let visibility = if repository.visibility.is_empty() {
            if repository.private {
                "private"
            } else {
                "public"
            }
        } else {
            &repository.visibility
        };
        if repository.owner.kind.eq_ignore_ascii_case("organization") {
            transaction.execute(
                "INSERT INTO github_organizations(organization_id,login,html_url,is_accessible,last_seen_at)
                 VALUES(?1,?2,?3,1,CURRENT_TIMESTAMP)
                 ON CONFLICT(organization_id) DO UPDATE SET login=excluded.login,
                    html_url=excluded.html_url,is_accessible=1,last_seen_at=CURRENT_TIMESTAMP",
                params![repository.owner.id, repository.owner.login, repository.owner.html_url],
            )?;
        }
        transaction.execute(
            "INSERT INTO github_repositories
             (repository_id, name, full_name, html_url, description, owner_id, owner_login,
              owner_url, owner_kind, visibility, archived, disabled, fork, is_template,
              default_branch, language, topics, size_kb, open_issues, effective_permission,
              created_at, updated_at, pushed_at, is_accessible, last_seen_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,
                     ?18,?19,?20,?21,?22,?23,1,CURRENT_TIMESTAMP)
             ON CONFLICT(repository_id) DO UPDATE SET
              name=excluded.name, full_name=excluded.full_name, html_url=excluded.html_url,
              description=excluded.description, owner_id=excluded.owner_id,
              owner_login=excluded.owner_login, owner_url=excluded.owner_url,
              owner_kind=excluded.owner_kind, visibility=excluded.visibility,
              archived=excluded.archived, disabled=excluded.disabled, fork=excluded.fork,
              is_template=excluded.is_template, default_branch=excluded.default_branch,
              language=excluded.language, topics=excluded.topics, size_kb=excluded.size_kb,
              open_issues=excluded.open_issues, effective_permission=excluded.effective_permission,
              created_at=excluded.created_at, updated_at=excluded.updated_at,
              pushed_at=excluded.pushed_at, is_accessible=1, last_seen_at=CURRENT_TIMESTAMP",
            params![
                repository.id,
                repository.name,
                repository.full_name,
                repository.html_url,
                repository.description.as_deref().unwrap_or(""),
                repository.owner.id,
                repository.owner.login,
                repository.owner.html_url,
                repository.owner.kind,
                visibility,
                repository.archived,
                repository.disabled,
                repository.fork,
                repository.is_template,
                repository.default_branch,
                repository.language.as_deref().unwrap_or(""),
                repository.topics.join(", "),
                repository.size as i64,
                repository.open_issues_count as i64,
                repository.effective_permission(),
                repository.created_at,
                repository.updated_at,
                repository.pushed_at,
            ],
        )?;
    }
    super::settings::set_in_transaction(
        &transaction,
        "github.last_sync_at",
        &current_timestamp(&transaction)?,
    )?;
    transaction.commit()?;
    Ok(())
}

fn current_timestamp(transaction: &rusqlite::Transaction<'_>) -> Result<String, rusqlite::Error> {
    transaction.query_row("SELECT CURRENT_TIMESTAMP", [], |row| row.get(0))
}

pub fn list(
    database: &Database,
    search: &str,
    owner: &str,
    visibility: &str,
    permission: &str,
    language: &str,
    size_filter: &str,
    pushed_filter: &str,
    tag: &str,
    include_inaccessible: bool,
    sort: &str,
    descending: bool,
) -> Result<Vec<RepositoryRow>, DatabaseError> {
    let (size_comparison, size_kb) = parse_count_filter(size_filter, "size", ">5000")?;
    let (pushed_comparison, pushed_value) = parse_date_filter(pushed_filter)?;
    let (exclude_tag, tag_slug) = tag
        .strip_prefix('!')
        .map_or((false, tag), |value| (true, value));
    let order = match sort {
        "owner" => "owner_login",
        "visibility" => "visibility",
        "size" => "size_kb",
        "permission" => "effective_permission",
        "language" => "language",
        "pushed" => "pushed_at",
        _ => "full_name",
    };
    let direction = if descending { "DESC" } else { "ASC" };
    let sql = format!(
        "SELECT repository_id,name,full_name,html_url,description,owner_login,owner_kind,
                visibility,archived,fork,language,size_kb,effective_permission,pushed_at
         FROM github_repositories r
         WHERE (?1 OR is_accessible=1)
           AND (?2='' OR full_name LIKE '%'||?2||'%' OR description LIKE '%'||?2||'%' OR topics LIKE '%'||?2||'%')
           AND (?3='' OR owner_login LIKE '%'||?3||'%')
           AND (?4='' OR visibility=?4)
           AND (?5='' OR effective_permission=?5)
           AND (?6='' OR language LIKE '%'||?6||'%')
           AND (?7=0 OR CASE ?7 WHEN 1 THEN size_kb>?8 WHEN 2 THEN size_kb>=?8 WHEN 3 THEN size_kb<?8 WHEN 4 THEN size_kb<=?8 ELSE size_kb=?8 END)
           AND (?9=0 OR CASE ?9 WHEN 1 THEN substr(pushed_at,1,10)>?10 WHEN 2 THEN substr(pushed_at,1,10)>=?10 WHEN 3 THEN substr(pushed_at,1,10)<?10 WHEN 4 THEN substr(pushed_at,1,10)<=?10 ELSE substr(pushed_at,1,10)=?10 END)
           AND (?11='' OR
                (?13=1 AND ((?12=0 AND NOT EXISTS(SELECT 1 FROM github_repository_tags rt WHERE rt.repository_id=r.repository_id))
                         OR (?12=1 AND EXISTS(SELECT 1 FROM github_repository_tags rt WHERE rt.repository_id=r.repository_id))))
                OR (?13=0 AND ((?12=0 AND EXISTS(SELECT 1 FROM github_repository_tags rt JOIN tags t ON t.id=rt.tag_id WHERE rt.repository_id=r.repository_id AND t.slug=?11))
                           OR (?12=1 AND NOT EXISTS(SELECT 1 FROM github_repository_tags rt JOIN tags t ON t.id=rt.tag_id WHERE rt.repository_id=r.repository_id AND t.slug=?11)))))
         ORDER BY {order} {direction}, repository_id {direction}"
    );
    let connection = database.connect()?;
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement
        .query_map(
            params![
                include_inaccessible,
                search.trim(),
                owner.trim(),
                visibility,
                permission,
                language.trim(),
                size_comparison,
                size_kb,
                pushed_comparison,
                pushed_value,
                tag_slug,
                exclude_tag,
                tag_slug == super::inventory::UNTAGGED_TAG_FILTER
            ],
            |row| {
                Ok(RepositoryRow {
                    repository_id: row.get(0)?,
                    name: row.get(1)?,
                    html_url: row.get(3)?,
                    description: row.get(4)?,
                    owner_login: row.get(5)?,
                    owner_kind: row.get(6)?,
                    visibility: row.get(7)?,
                    archived: row.get(8)?,
                    fork: row.get(9)?,
                    language: row.get(10)?,
                    size_kb: row.get::<_, i64>(11)? as u64,
                    effective_permission: row.get(12)?,
                    pushed_at: row.get(13)?,
                    tags: Vec::new(),
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut tag_statement = connection.prepare(
        "SELECT t.slug,t.name,t.description,t.color,
                0,0,0,0,1,0 FROM github_repository_tags rt JOIN tags t ON t.id=rt.tag_id
         WHERE rt.repository_id=?1 ORDER BY t.name COLLATE NOCASE",
    )?;
    for repository in &mut rows {
        repository.tags = tag_statement
            .query_map([repository.repository_id], |row| {
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
    Ok(rows)
}

fn comparison_prefix(value: &str) -> (i64, &str) {
    for (prefix, comparison) in [(">=", 2), ("<=", 4), (">", 1), ("<", 3), ("=", 5)] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return (comparison, rest.trim());
        }
    }
    (5, value.trim())
}

fn parse_count_filter(
    filter: &str,
    label: &str,
    example: &str,
) -> Result<(i64, i64), DatabaseError> {
    let filter = filter.trim();
    if filter.is_empty() {
        return Ok((0, 0));
    }
    let (comparison, value) = comparison_prefix(filter);
    let normalized = value.to_ascii_uppercase();
    let value = if let Some(number) = normalized.strip_suffix("KB") {
        number.trim()
    } else if normalized
        .chars()
        .any(|character| character.is_ascii_alphabetic())
    {
        return Err(format!(
            "Invalid {label} filter '{filter}'. GitHub sizes use KB; try {example}."
        )
        .into());
    } else {
        normalized.trim()
    };
    let value = value
        .parse::<i64>()
        .map_err(|_| format!("Invalid {label} filter '{filter}'. Try {example}."))?;
    if value < 0 {
        return Err(format!("GitHub repository {label} cannot be negative").into());
    }
    Ok((comparison, value))
}

fn parse_date_filter(filter: &str) -> Result<(i64, String), DatabaseError> {
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
        return Err(format!("Invalid last-push filter '{filter}'. Try >2026-01-01.").into());
    }
    Ok((comparison, value.to_string()))
}

pub fn change_tags(
    database: &Database,
    repository_ids: &[i64],
    slug: &str,
    remove: bool,
) -> Result<usize, DatabaseError> {
    if repository_ids.is_empty() {
        return Err("Select at least one GitHub repository".into());
    }
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let tag_id: i64 = transaction.query_row(
        if remove { "SELECT id FROM tags WHERE slug=?1" } else {
            "SELECT t.id FROM tags t JOIN tag_scopes s ON s.tag_id=t.id WHERE t.slug=?1 AND s.scope='github-repositories'"
        }, [slug], |row| row.get(0)
    ).optional()?.ok_or_else(|| format!("Tag '{slug}' is not available for GitHub repositories"))?;
    let mut changed = 0;
    for id in repository_ids {
        changed += if remove {
            transaction.execute(
                "DELETE FROM github_repository_tags WHERE repository_id=?1 AND tag_id=?2",
                params![id, tag_id],
            )?
        } else {
            transaction.execute("INSERT OR IGNORE INTO github_repository_tags(repository_id,tag_id) SELECT repository_id,?2 FROM github_repositories WHERE repository_id=?1", params![id,tag_id])?
        };
    }
    transaction.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::format_repository_size;

    #[test]
    fn formats_repository_sizes_with_adaptive_units() {
        assert_eq!(format_repository_size(2208), "2.2 MB");
        assert_eq!(format_repository_size(2_208_400), "2.2 GB");
        assert_eq!(format_repository_size(2_208_400_000), "2.2 TB");
        assert_eq!(format_repository_size(220), "220.0 KB");
        assert_eq!(format_repository_size(0), "0.0 KB");
    }
}
