use super::{Database, DatabaseError, inventory::Tag};
use crate::local_files::Item;
use rusqlite::params;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Row {
    pub id: i64,
    pub root_path: String,
    pub relative_path: String,
    pub name: String,
    pub extension: String,
    pub type_label: String,
    pub is_directory: bool,
    pub size_bytes: u64,
    pub size_label: String,
    pub modified_unix: i64,
    pub modified_label: String,
    pub checksum_sha256: String,
    pub is_symlink: bool,
    pub symlink_target: String,
    pub full_path: String,
    pub icon_class: &'static str,
    pub owner_username: String,
    pub owner_identifier: String,
    pub owner_principal_id: i64,
    pub owner_display_name: String,
    pub group_name: String,
    pub group_identifier: String,
    pub duplicate_copies: u64,
    pub tags: Vec<Tag>,
}
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    pub size_label: String,
    pub duplicate_groups: u64,
    pub duplicate_bytes: u64,
    pub duplicate_size_label: String,
    pub completed_at: String,
}
pub fn checksum_cache(
    db: &Database,
) -> Result<HashMap<(String, String), (u64, i64, String)>, DatabaseError> {
    let c = db.connect()?;
    let mut s=c.prepare("SELECT root_path,relative_path,size_bytes,modified_unix,checksum_sha256 FROM local_file_items WHERE checksum_sha256<>''")?;
    let rows = s.query_map([], |r| {
        Ok((
            (r.get(0)?, r.get(1)?),
            (r.get::<_, i64>(2)? as u64, r.get(3)?, r.get(4)?),
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
#[cfg(test)]
pub fn synchronize(db: &Database, items: &[Item]) -> Result<(), DatabaseError> {
    synchronize_inner(db, items, None)
}

pub fn synchronize_cancellable(
    db: &Database,
    items: &[Item],
    cancellation: &Arc<AtomicBool>,
) -> Result<(), DatabaseError> {
    synchronize_inner(db, items, Some(cancellation))
}

fn synchronize_inner(
    db: &Database,
    items: &[Item],
    cancellation: Option<&Arc<AtomicBool>>,
) -> Result<(), DatabaseError> {
    let mut c = db.connect()?;
    let tx = c.transaction()?;
    tx.execute("UPDATE local_file_items SET is_accessible=0", [])?;
    {
        let mut statement = tx.prepare("INSERT INTO local_file_items(root_path,relative_path,name,extension,is_directory,size_bytes,modified_unix,checksum_sha256,owner_username,owner_identifier,group_name,group_identifier,is_symlink,symlink_target,is_accessible,last_seen_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,1,CURRENT_TIMESTAMP) ON CONFLICT(root_path,relative_path) DO UPDATE SET name=excluded.name,extension=excluded.extension,is_directory=excluded.is_directory,size_bytes=excluded.size_bytes,modified_unix=excluded.modified_unix,checksum_sha256=excluded.checksum_sha256,owner_username=excluded.owner_username,owner_identifier=excluded.owner_identifier,group_name=excluded.group_name,group_identifier=excluded.group_identifier,is_symlink=excluded.is_symlink,symlink_target=excluded.symlink_target,is_accessible=1,last_seen_at=CURRENT_TIMESTAMP")?;
        for i in items {
            if cancellation.is_some_and(|value| value.load(std::sync::atomic::Ordering::Acquire)) {
                return Err("Local Files database update cancelled".into());
            }
            statement.execute(params![
                i.root_path,
                i.relative_path,
                i.name,
                i.extension,
                i.is_directory,
                i.size_bytes as i64,
                i.modified_unix,
                i.checksum_sha256,
                i.owner_username,
                i.owner_identifier,
                i.group_name,
                i.group_identifier,
                i.is_symlink,
                i.symlink_target
            ])?;
        }
    }
    tx.execute("INSERT INTO settings(key,value,updated_at) VALUES('local_files.last_sync_at',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP) ON CONFLICT(key) DO UPDATE SET value=CURRENT_TIMESTAMP,updated_at=CURRENT_TIMESTAMP",[])?;
    tx.commit()?;
    Ok(())
}
pub fn list_children(
    db: &Database,
    root: &str,
    parent: &str,
    search: &str,
    name: &str,
    path: &str,
    item_type: &str,
    size: &str,
    modified: &str,
    owner: &str,
    group: &str,
    tag: &str,
    duplicates_only: bool,
    sort: &str,
    descending: bool,
) -> Result<Vec<Row>, DatabaseError> {
    let c = db.connect()?;
    let (modified_comparison, modified_value) = parse_modified_filter(modified)?;
    let order = match sort {
        "path" => "i.relative_path",
        "type" => "i.is_directory DESC,i.extension",
        "size" => "i.size_bytes",
        "modified" => "i.modified_unix",
        "owner" => "i.owner_username COLLATE NOCASE",
        "group" => "i.group_name COLLATE NOCASE",
        "duplicates" => "copies",
        _ => "i.name COLLATE NOCASE",
    };
    let direction = if descending { "DESC" } else { "ASC" };
    let type_expression = "CASE WHEN i.is_symlink=1 THEN 'symlink' WHEN i.is_directory=1 THEN 'folder' WHEN i.extension<>'' THEN i.extension ELSE 'file' END";
    let size_expression = "CASE WHEN i.size_bytes>=1000000000000 THEN printf('%.1f TB',i.size_bytes/1000000000000.0) WHEN i.size_bytes>=1000000000 THEN printf('%.1f GB',i.size_bytes/1000000000.0) WHEN i.size_bytes>=1000000 THEN printf('%.1f MB',i.size_bytes/1000000.0) WHEN i.size_bytes>=1000 THEN printf('%.1f KB',i.size_bytes/1000.0) ELSE CAST(i.size_bytes AS TEXT)||' B' END";
    let sql = format!(
        "SELECT i.id,i.root_path,i.relative_path,i.name,i.extension,i.is_directory,i.size_bytes,i.modified_unix,i.checksum_sha256,CASE WHEN i.checksum_sha256='' THEN 0 ELSE (SELECT COUNT(*) FROM local_file_items d WHERE d.is_accessible=1 AND d.checksum_sha256=i.checksum_sha256) END copies,(SELECT group_concat(t.slug||char(30)||t.name||char(30)||t.color||char(30)||t.description,char(31)) FROM local_file_tags lft JOIN tags t ON t.id=lft.tag_id WHERE lft.local_file_id=i.id),i.owner_username,i.owner_identifier,COALESCE(p.id,0),COALESCE(p.display_name,''),i.group_name,i.group_identifier,i.is_symlink,i.symlink_target FROM local_file_items i LEFT JOIN principals p ON lower(p.username)=lower(i.owner_username) WHERE i.is_accessible=1 AND i.root_path=?1 AND ((?10='' AND ((?2='' AND instr(i.relative_path,'/')=0) OR (?2<>'' AND i.relative_path LIKE ?2||'/%' AND instr(substr(i.relative_path,length(?2)+2),'/')=0))) OR (?10<>'' AND (instr(lower(i.name),lower(?10))>0 OR instr(lower(i.relative_path),lower(?10))>0 OR instr(lower({type_expression}),lower(?10))>0 OR instr(lower(CAST(i.size_bytes AS TEXT)),lower(?10))>0 OR instr(lower({size_expression}),lower(?10))>0 OR instr(lower(replace(datetime(i.modified_unix,'unixepoch','localtime'),' ','T')),lower(?10))>0 OR instr(lower(i.owner_username),lower(?10))>0 OR instr(lower(i.owner_identifier),lower(?10))>0 OR instr(lower(COALESCE(p.display_name,'')),lower(?10))>0 OR instr(lower(i.group_name),lower(?10))>0 OR instr(lower(i.group_identifier),lower(?10))>0))) AND (?3='' OR instr(lower(i.name),lower(?3))>0) AND (?4='' OR instr(lower(i.relative_path),lower(?4))>0) AND (?5='' OR instr(lower({type_expression}),lower(?5))>0) AND (?6='' OR instr(lower(CAST(i.size_bytes AS TEXT)),lower(?6))>0 OR instr(lower({size_expression}),lower(?6))>0) AND (?11=0 OR (?11=1 AND datetime(i.modified_unix,'unixepoch','localtime') > replace(?7,'T',' ')) OR (?11=2 AND datetime(i.modified_unix,'unixepoch','localtime') >= replace(?7,'T',' ')) OR (?11=3 AND datetime(i.modified_unix,'unixepoch','localtime') < replace(?7,'T',' ')) OR (?11=4 AND datetime(i.modified_unix,'unixepoch','localtime') <= replace(?7,'T',' ')) OR (?11=5 AND substr(datetime(i.modified_unix,'unixepoch','localtime'),1,length(?7))=replace(?7,'T',' '))) AND (?12='' OR instr(lower(i.owner_username),lower(?12))>0 OR instr(lower(i.owner_identifier),lower(?12))>0 OR instr(lower(COALESCE(p.display_name,'')),lower(?12))>0) AND (?13='' OR instr(lower(i.group_name),lower(?13))>0 OR instr(lower(i.group_identifier),lower(?13))>0) AND (?8='' OR (?8='__untagged__' AND NOT EXISTS(SELECT 1 FROM local_file_tags x WHERE x.local_file_id=i.id)) OR EXISTS(SELECT 1 FROM local_file_tags x JOIN tags xt ON xt.id=x.tag_id WHERE x.local_file_id=i.id AND xt.slug=?8)) AND (?9=0 OR (i.checksum_sha256<>'' AND (SELECT COUNT(*) FROM local_file_items d WHERE d.is_accessible=1 AND d.checksum_sha256=i.checksum_sha256)>1)) ORDER BY i.is_directory DESC,{order} {direction}"
    );
    let mut s = c.prepare(&sql)?;
    let rows = s.query_map(
        params![
            root,
            parent,
            name,
            path,
            item_type,
            size,
            modified_value,
            tag,
            duplicates_only,
            search,
            modified_comparison,
            owner,
            group
        ],
        |r| {
            let bytes = r.get::<_, i64>(6)? as u64;
            let root_path: String = r.get(1)?;
            let relative_path: String = r.get(2)?;
            let extension: String = r.get(4)?;
            let is_directory: bool = r.get(5)?;
            Ok(Row {
                id: r.get(0)?,
                full_path: Path::new(&root_path)
                    .join(&relative_path)
                    .to_string_lossy()
                    .into_owned(),
                root_path,
                relative_path,
                name: r.get(3)?,
                icon_class: file_icon_class(&extension),
                type_label: item_type_label(is_directory, r.get(17)?, &extension),
                extension,
                is_directory,
                size_bytes: bytes,
                size_label: format_bytes(bytes),
                modified_unix: r.get(7)?,
                modified_label: r.get::<_, i64>(7).map(|v| format_unix(v))?,
                checksum_sha256: r.get(8)?,
                duplicate_copies: r.get::<_, i64>(9)? as u64,
                tags: parse_tags(r.get::<_, Option<String>>(10)?),
                owner_username: r.get(11)?,
                owner_identifier: r.get(12)?,
                owner_principal_id: r.get(13)?,
                owner_display_name: r.get(14)?,
                group_name: r.get(15)?,
                group_identifier: r.get(16)?,
                is_symlink: r.get(17)?,
                symlink_target: r.get(18)?,
            })
        },
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn item_type_label(is_directory: bool, is_symlink: bool, extension: &str) -> String {
    if is_symlink {
        return "Symlink".to_string();
    }
    if is_directory {
        "Folder".to_string()
    } else if extension.is_empty() {
        "File".to_string()
    } else {
        extension.to_ascii_uppercase()
    }
}

fn parse_modified_filter(filter: &str) -> Result<(i64, String), DatabaseError> {
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
    let date = value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| index == 4 || index == 7 || character.is_ascii_digit());
    let timestamp = value.len() == 19
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && matches!(value.as_bytes()[10], b'T' | b' ')
        && value.as_bytes()[13] == b':'
        && value.as_bytes()[16] == b':'
        && value.chars().enumerate().all(|(index, character)| {
            matches!(index, 4 | 7 | 10 | 13 | 16) || character.is_ascii_digit()
        });
    if !date && !timestamp {
        return Err(format!(
            "Invalid modified-time filter '{filter}'. Try >2026-01-01 or <=2026-01-01T12:00:00."
        )
        .into());
    }
    Ok((comparison, value.to_string()))
}

pub fn change_tags(
    database: &Database,
    ids: &[i64],
    slug: &str,
    remove: bool,
) -> Result<usize, DatabaseError> {
    if ids.is_empty() {
        return Err("Select at least one local file or folder".into());
    }
    let mut c = database.connect()?;
    let tx = c.transaction()?;
    let tag_id:i64=tx.query_row("SELECT t.id FROM tags t JOIN tag_scopes s ON s.tag_id=t.id WHERE t.slug=?1 AND s.scope='local-files'",[slug],|r|r.get(0)).map_err(|_|format!("Tag '{slug}' is not available for Local Files"))?;
    let mut changed = 0;
    for id in ids {
        changed += if remove {
            tx.execute(
                "DELETE FROM local_file_tags WHERE local_file_id=?1 AND tag_id=?2",
                params![id, tag_id],
            )?
        } else {
            tx.execute("INSERT OR IGNORE INTO local_file_tags(local_file_id,tag_id) SELECT id,?2 FROM local_file_items WHERE id=?1",params![id,tag_id])?
        };
    }
    tx.commit()?;
    Ok(changed)
}

fn parse_tags(value: Option<String>) -> Vec<Tag> {
    value
        .unwrap_or_default()
        .split('\u{1f}')
        .filter_map(|v| {
            let mut p = v.split('\u{1e}');
            Some(Tag {
                slug: p.next()?.into(),
                name: p.next()?.into(),
                color: p.next()?.into(),
                description: p.next()?.into(),
                directory: false,
                my_drive: false,
                shared_drives: false,
                shared_with_me: false,
                github_repositories: false,
                keeper_shared_folders: false,
                local_files: true,
            })
        })
        .collect()
}
fn format_unix(value: i64) -> String {
    if value <= 0 {
        String::new()
    } else {
        local_iso_datetime(value)
    }
}

#[cfg(unix)]
fn local_iso_datetime(value: i64) -> String {
    let timestamp = value as libc::time_t;
    let mut local = unsafe { std::mem::zeroed::<libc::tm>() };
    if unsafe { libc::localtime_r(&timestamp, &mut local) }.is_null() {
        return String::new();
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        local.tm_year + 1900,
        local.tm_mon + 1,
        local.tm_mday,
        local.tm_hour,
        local.tm_min,
        local.tm_sec
    )
}

#[cfg(windows)]
fn local_iso_datetime(value: i64) -> String {
    let timestamp = value as libc::time_t;
    let mut local = unsafe { std::mem::zeroed::<libc::tm>() };
    if unsafe { libc::localtime_s(&mut local, &timestamp) } != 0 {
        return String::new();
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        local.tm_year + 1900,
        local.tm_mon + 1,
        local.tm_mday,
        local.tm_hour,
        local.tm_min,
        local.tm_sec
    )
}

fn file_icon_class(extension: &str) -> &'static str {
    match extension.to_ascii_lowercase().as_str() {
        "pdf" => "bi-file-earmark-pdf text-danger",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "tif" | "tiff" | "bmp" => {
            "bi-file-earmark-image text-success"
        }
        "mp3" | "wav" | "flac" | "m4a" | "ogg" => "bi-file-earmark-music text-primary",
        "mp4" | "mov" | "mkv" | "avi" | "webm" => "bi-file-earmark-play text-primary",
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" => "bi-file-earmark-zip text-warning",
        "xls" | "xlsx" | "ods" | "csv" => "bi-file-earmark-excel text-success",
        "doc" | "docx" | "odt" | "rtf" => "bi-file-earmark-word text-primary",
        "ppt" | "pptx" | "odp" => "bi-file-earmark-slides text-danger",
        "rs" | "py" | "js" | "ts" | "html" | "css" | "json" | "toml" | "yaml" | "yml" | "xml"
        | "sh" => "bi-file-earmark-code text-primary",
        "txt" | "md" | "log" => "bi-file-earmark-text",
        _ => "bi-file-earmark",
    }
}
pub fn summary(db: &Database) -> Result<Summary, DatabaseError> {
    let c = db.connect()?;
    let mut summary=c.query_row("SELECT COALESCE(SUM(CASE WHEN is_directory=0 THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN is_directory=1 THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN is_directory=0 THEN size_bytes ELSE 0 END),0),(SELECT COUNT(*) FROM (SELECT checksum_sha256 FROM local_file_items WHERE is_accessible=1 AND checksum_sha256<>'' GROUP BY checksum_sha256 HAVING COUNT(*)>1)),(SELECT COALESCE(SUM((copies-1)*size_bytes),0) FROM (SELECT size_bytes,COUNT(*) copies FROM local_file_items WHERE is_accessible=1 AND checksum_sha256<>'' GROUP BY checksum_sha256,size_bytes HAVING COUNT(*)>1)),COALESCE((SELECT value FROM settings WHERE key='local_files.last_sync_at'),'') FROM local_file_items WHERE is_accessible=1",[],|r|Ok(Summary{files:r.get::<_,i64>(0)? as u64,folders:r.get::<_,i64>(1)? as u64,bytes:r.get::<_,i64>(2)? as u64,size_label:String::new(),duplicate_groups:r.get::<_,i64>(3)? as u64,duplicate_bytes:r.get::<_,i64>(4)? as u64,duplicate_size_label:String::new(),completed_at:r.get(5)?}))?;
    summary.size_label = format_bytes(summary.bytes);
    summary.duplicate_size_label = format_bytes(summary.duplicate_bytes);
    Ok(summary)
}

fn format_bytes(bytes: u64) -> String {
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
    format!("{bytes} B")
}

#[cfg(test)]
mod tests {
    use super::{file_icon_class, format_bytes, format_unix, parse_modified_filter};
    #[test]
    fn formats_local_file_sizes_with_adaptive_units() {
        assert_eq!(format_bytes(2_208_400_000_000), "2.2 TB");
        assert_eq!(format_bytes(1_250_000), "1.2 MB");
        assert_eq!(format_bytes(42), "42 B");
    }

    #[test]
    fn formats_local_timestamps_and_file_icons() {
        let timestamp = format_unix(1_788_243_845);
        assert_eq!(timestamp.len(), 19);
        assert_eq!(timestamp.as_bytes()[4], b'-');
        assert_eq!(timestamp.as_bytes()[10], b'T');
        assert_eq!(file_icon_class("PDF"), "bi-file-earmark-pdf text-danger");
        assert_eq!(file_icon_class("jpg"), "bi-file-earmark-image text-success");
    }

    #[test]
    fn parses_before_and_after_modified_time_filters() {
        assert_eq!(
            parse_modified_filter(">2026-01-01").expect("date filter should parse"),
            (1, "2026-01-01".to_string())
        );
        assert_eq!(
            parse_modified_filter("<=2026-01-01T12:30:45").expect("timestamp filter should parse"),
            (4, "2026-01-01T12:30:45".to_string())
        );
        assert!(parse_modified_filter("tomorrow").is_err());
    }
}
