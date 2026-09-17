use super::{RcloneError, command, config};
use crate::bootstrap::Runtime;
use crate::database::{self, local_migration::Entry, migration::MigrationSource};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn validate_sources(
    database: &database::Database,
    job: i64,
    sources: &[MigrationSource],
) -> Result<(), RcloneError> {
    for source in sources {
        let entries = database::local_migration::entries(database, job, &source.item_id)?;
        if entries.is_empty() {
            return Err("The local migration has no saved file list. Create a new plan.".into());
        }
        for entry in entries {
            checked_path(&entry)?;
        }
    }
    Ok(())
}

// Do not follow symlinks introduced since indexing, including intermediate
// directories. The configured root itself may be a user-selected symlink.
fn checked_path(entry: &Entry) -> Result<PathBuf, RcloneError> {
    database::local_migration::validate_relative(&entry.relative_path)?;
    if !Path::new(&entry.root_path).is_absolute() {
        return Err("Invalid local migration root".into());
    }
    let mut path = fs::canonicalize(&entry.root_path)?;
    for component in entry.relative_path.split('/') {
        path.push(component);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(format!(
                "Local source became a symbolic link: {}. Reindex and create a new plan.",
                path.display()
            )
            .into());
        }
    }
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.is_dir() != entry.is_directory || (!entry.is_directory && !metadata.is_file()) {
        return Err(format!(
            "Local source changed type: {}. Reindex and create a new plan.",
            path.display()
        )
        .into());
    }
    Ok(path)
}

pub fn copy_source(
    runtime: &Runtime,
    executable: &Path,
    database: &database::Database,
    job: i64,
    source: &MigrationSource,
    drive: &str,
    folder: &str,
    immutable: bool,
) -> Result<(), RcloneError> {
    let entries = database::local_migration::entries(database, job, &source.item_id)?;
    let destination = format!(
        "{}{}",
        super::migration::destination_remote(drive, folder),
        source.name
    );
    copy_selection(
        executable,
        &config::path(runtime)?,
        source,
        &entries,
        &destination,
        immutable,
    )
}

fn copy_selection(
    executable: &Path,
    config: &Path,
    source: &MigrationSource,
    entries: &[Entry],
    destination: &str,
    immutable: bool,
) -> Result<(), RcloneError> {
    if entries.is_empty() {
        return Err("Local migration file list is empty".into());
    }
    let paths = entries
        .iter()
        .map(checked_path)
        .collect::<Result<Vec<_>, _>>()?;
    let source_path = fs::canonicalize(&source.relative_path)?;
    if !entries
        .iter()
        .zip(&paths)
        .any(|(entry, path)| *path == source_path && entry.is_directory == source.is_directory)
    {
        return Err("The selected local source is missing from its saved file list".into());
    }
    let common = vec![
        "--config".to_string(),
        config.to_string_lossy().into_owned(),
        "--copy-links=false".into(),
        "--links=false".into(),
    ];
    if !source.is_directory {
        let mut args = vec![
            "copyto".into(),
            source_path.to_string_lossy().into_owned(),
            destination.into(),
        ];
        args.extend(common.clone());
        if immutable {
            args.push("--immutable".into());
        }
        run(executable, &args)?;
    } else {
        let mut files = Vec::new();
        let mut directories = Vec::new();
        let mut nonempty = HashSet::new();
        for (entry, path) in entries.iter().zip(&paths) {
            let relative = path
                .strip_prefix(&source_path)
                .map_err(|_| "A saved file is outside the selected folder")?;
            if let Some(parent) = relative.parent() {
                nonempty.insert(parent.to_path_buf());
            }
            if entry.is_directory {
                directories.push(relative.to_path_buf());
            } else {
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
        if !files.is_empty() {
            let manifest = Manifest::new(&files)?;
            let mut args = vec![
                "copy".into(),
                source_path.to_string_lossy().into_owned(),
                destination.into(),
                "--files-from0".into(),
                manifest.0.to_string_lossy().into_owned(),
                "--no-traverse".into(),
            ];
            args.extend(common.clone());
            if immutable {
                args.push("--immutable".into());
            }
            run(executable, &args)?;
        }
        // Only empty indexed leaf folders need explicit creation. Copy creates
        // parents for files; mkdir creates the ancestors of each empty leaf.
        for directory in directories.iter().filter(|d| !nonempty.contains(*d)) {
            let suffix = directory.to_string_lossy().replace('\\', "/");
            let target = if suffix.is_empty() {
                destination.to_string()
            } else {
                format!("{}/{suffix}", destination.trim_end_matches('/'))
            };
            let mut args = vec!["mkdir".into(), target];
            args.extend(common.clone());
            run(executable, &args)?;
        }
    }
    // Rclone file lists ignore missing source paths. Detect removals during the
    // copy before recording completion, so a partial upload can be resumed.
    for entry in entries {
        checked_path(entry)?;
    }
    Ok(())
}

fn run(executable: &Path, args: &[String]) -> Result<(), RcloneError> {
    let output = command::run(executable, args)?;
    if !output.status.success() {
        return Err(format!(
            "Local upload failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(())
}

struct Manifest(PathBuf);
impl Manifest {
    fn new(files: &[String]) -> Result<Self, RcloneError> {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|e| format!("Unable to create transfer file list: {e}"))?;
        let name = random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let path = std::env::temp_dir().join(format!("boreal-upload-{name}"));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        let manifest = Self(path);
        for path in files {
            file.write_all(path.as_bytes())?;
            file.write_all(&[0])?;
        }
        file.flush()?;
        Ok(manifest)
    }
}
impl Drop for Manifest {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temporary() -> PathBuf {
        std::env::temp_dir().join(format!(
            "boreal-local-upload-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn local_upload_rejects_missing_sources_and_symlink_ancestors() {
        let root = temporary();
        fs::create_dir_all(root.join("folder")).unwrap();
        let entry = Entry {
            root_path: root.to_string_lossy().into_owned(),
            relative_path: "folder/missing".into(),
            is_directory: false,
        };
        assert!(checked_path(&entry).is_err());
        #[cfg(unix)]
        {
            fs::create_dir_all(root.join("elsewhere")).unwrap();
            fs::write(root.join("elsewhere/file"), b"not selected").unwrap();
            std::os::unix::fs::symlink(root.join("elsewhere"), root.join("folder/link")).unwrap();
            let entry = Entry {
                relative_path: "folder/link/file".into(),
                ..entry
            };
            assert!(
                checked_path(&entry)
                    .unwrap_err()
                    .to_string()
                    .contains("symbolic link")
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "requires BOREAL_TEST_RCLONE; copies only between temporary local folders"]
    fn local_upload_manifest_copies_only_indexed_files_and_resumes_without_deleting() {
        let executable = PathBuf::from(
            std::env::var("BOREAL_TEST_RCLONE").expect("provide managed rclone path"),
        );
        let root = temporary();
        fs::create_dir_all(root.join("source/empty/nested")).unwrap();
        fs::write(root.join("source/#file\nwith space.txt"), b"selected").unwrap();
        fs::write(root.join("source/excluded.txt"), b"not selected").unwrap();
        let paths = [
            ("source", true),
            ("source/empty", true),
            ("source/empty/nested", true),
            ("source/#file\nwith space.txt", false),
        ];
        let entries = paths
            .iter()
            .map(|(p, d)| Entry {
                root_path: root.to_string_lossy().into_owned(),
                relative_path: p.to_string(),
                is_directory: *d,
            })
            .collect::<Vec<_>>();
        let source = MigrationSource {
            item_id: "1".into(),
            name: "source".into(),
            relative_path: root.join("source").to_string_lossy().into_owned(),
            is_directory: true,
            files_total: 1,
            folders_total: 3,
            bytes_total: 8,
            status: "pending".into(),
            error_message: String::new(),
        };
        let config = root.join("rclone.conf");
        fs::write(&config, "").unwrap();
        let dest = root.join("destination");
        copy_selection(
            &executable,
            &config,
            &source,
            &entries,
            &dest.to_string_lossy(),
            true,
        )
        .unwrap();
        assert_eq!(
            fs::read(dest.join("#file\nwith space.txt")).unwrap(),
            b"selected"
        );
        assert!(!dest.join("excluded.txt").exists());
        assert!(dest.join("empty/nested").is_dir());
        fs::write(
            root.join("source/#file\nwith space.txt"),
            b"updated contents",
        )
        .unwrap();
        fs::write(dest.join("destination-only.txt"), b"keep").unwrap();
        assert!(
            copy_selection(
                &executable,
                &config,
                &source,
                &entries,
                &dest.to_string_lossy(),
                true
            )
            .is_err()
        );
        copy_selection(
            &executable,
            &config,
            &source,
            &entries,
            &dest.to_string_lossy(),
            false,
        )
        .unwrap();
        assert_eq!(
            fs::read(dest.join("#file\nwith space.txt")).unwrap(),
            b"updated contents"
        );
        assert!(dest.join("destination-only.txt").exists());
        assert!(root.join("source/excluded.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
