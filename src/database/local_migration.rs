use super::{Database, DatabaseError};
use rusqlite::{OptionalExtension, params};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Entry {
    pub root_path: String,
    pub relative_path: String,
    pub is_directory: bool,
}

struct Selection {
    id: i64,
    root: String,
    path: String,
    name: String,
    directory: bool,
}
impl Selection {
    fn full_path(&self) -> PathBuf {
        Path::new(&self.root).join(&self.path)
    }
}

/// Freeze the indexed selection, rather than recursively uploading whatever
/// happens to be present in the selected directories when the job starts.
pub fn create(database: &Database, ids: &[i64]) -> Result<i64, DatabaseError> {
    if ids.is_empty() {
        return Err("Select at least one local file or folder".into());
    }
    let settings = super::settings::load(database)?;
    let roots: HashSet<_> = crate::local_files::parse_roots(&settings.local_file_roots)
        .into_iter()
        .collect();
    let mut c = database.connect()?;
    let tx = c.transaction()?;
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            continue;
        }
        let (entry, symlink) = tx.query_row("SELECT id,root_path,relative_path,name,is_directory,is_symlink FROM local_file_items WHERE id=?1 AND is_accessible=1", [id], |r| Ok((Selection { id:r.get(0)?,root:r.get(1)?,path:r.get(2)?,name:r.get(3)?,directory:r.get(4)? }, r.get::<_,bool>(5)?))).optional()?.ok_or("A selected local entry is no longer indexed. Refresh the explorer.")?;
        if !roots.contains(Path::new(&entry.root)) {
            return Err("A selected root is no longer configured. Refresh the explorer.".into());
        }
        if symlink {
            return Err(
                "Symbolic links cannot be migrated. Select the original indexed files instead."
                    .into(),
            );
        }
        validate_relative(&entry.path)?;
        if entry.name.is_empty()
            || entry.name == "."
            || entry.name == ".."
            || entry.name.contains(['/', '\\', '\0'])
        {
            return Err("Invalid local source name".into());
        }
        selected.push(entry);
    }
    selected.sort_by_key(|s| s.full_path());
    let mut top: Vec<Selection> = Vec::new();
    let mut selected_paths = HashSet::new();
    let mut selected_directories = HashSet::new();
    for entry in selected {
        let full = entry.full_path();
        if selected_paths.contains(&full)
            || full
                .ancestors()
                .skip(1)
                .any(|parent| selected_directories.contains(parent))
        {
            continue;
        }
        selected_paths.insert(full.clone());
        if entry.directory {
            selected_directories.insert(full);
        }
        top.push(entry);
    }
    let mut names = HashSet::new();
    for entry in &top {
        if !names.insert(entry.name.clone()) {
            return Err(format!("Multiple selected items are named '{}'. Migrate them separately or select their parent folders.", entry.name).into());
        }
    }
    tx.execute("INSERT INTO migration_jobs(source_scope,source_kind,operation_kind) VALUES('local-files','local-files','drive-copy')", [])?;
    let job = tx.last_insert_rowid();
    for source in &top {
        let id = source.id.to_string();
        tx.execute("INSERT INTO migration_sources(migration_id,item_id,name,relative_path,is_directory) VALUES(?1,?2,?3,?4,?5)", params![job,id,source.name,source.full_path().to_string_lossy(),source.directory])?;
        // Both branches use the existing (root_path,relative_path) unique index.
        tx.execute("INSERT INTO local_migration_entries(migration_id,source_id,root_path,relative_path,is_directory,size_bytes) SELECT ?1,?2,root_path,relative_path,is_directory,size_bytes FROM local_file_items WHERE id=?3 AND is_accessible=1 AND is_symlink=0", params![job,id,source.id])?;
        if source.directory {
            tx.execute("INSERT INTO local_migration_entries(migration_id,source_id,root_path,relative_path,is_directory,size_bytes) SELECT ?1,?2,root_path,relative_path,is_directory,size_bytes FROM local_file_items WHERE root_path=?3 AND relative_path>=?4||'/' AND relative_path<?4||'0' AND is_accessible=1 AND is_symlink=0", params![job,id,source.root,source.path])?;
        }
        tx.execute("UPDATE migration_sources SET (files_total,folders_total,bytes_total)=(SELECT COALESCE(SUM(is_directory=0),0),COALESCE(SUM(is_directory=1),0),COALESCE(SUM(CASE WHEN is_directory=0 THEN size_bytes ELSE 0 END),0) FROM local_migration_entries WHERE migration_id=?1 AND source_id=?2) WHERE migration_id=?1 AND item_id=?2", params![job,id])?;
    }
    tx.execute("UPDATE migration_jobs SET (files_total,folders_total,bytes_total)=(SELECT SUM(files_total),SUM(folders_total),SUM(bytes_total) FROM migration_sources WHERE migration_id=?1) WHERE id=?1", [job])?;
    tx.commit()?;
    Ok(job)
}

pub fn entries(database: &Database, job: i64, source: &str) -> Result<Vec<Entry>, DatabaseError> {
    let c = database.connect()?;
    let mut s = c.prepare("SELECT root_path,relative_path,is_directory FROM local_migration_entries WHERE migration_id=?1 AND source_id=?2 ORDER BY relative_path")?;
    Ok(s.query_map(params![job, source], |r| {
        Ok(Entry {
            root_path: r.get(0)?,
            relative_path: r.get(1)?,
            is_directory: r.get(2)?,
        })
    })?
    .collect::<Result<_, _>>()?)
}

pub fn validate_relative(path: &str) -> Result<(), DatabaseError> {
    if path.is_empty()
        || path.contains(['\0', '\\'])
        || Path::new(path).is_absolute()
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("Invalid indexed local path; update Local Files metadata".into());
    }
    Ok(())
}
