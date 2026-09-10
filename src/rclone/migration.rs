use std::{collections::HashSet, path::Path};

use serde::Deserialize;

use crate::{bootstrap::Runtime, database::migration::MigrationSource};

use super::{RcloneError, command, config, identity, inventory, remotes::RemoteKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedDriveDestination {
    pub drive_id: String,
    pub drive_name: String,
    pub folder_id: String,
    pub folder_name: String,
    pub folders: Vec<identity::GoogleDriveFolder>,
}

/// Validate that a folder ID is accessible through the read-only remote and
/// belongs to one of the account's Shared Drives. This performs only a Shared
/// Drive listing and one exact Google Drive metadata lookup authenticated by
/// the Rclone-managed token; it does not build an inventory.
pub fn validate_destination(
    runtime: &Runtime,
    executable: &Path,
    folder_id: &str,
    require_shared_drive: bool,
) -> Result<SharedDriveDestination, RcloneError> {
    // Contact Drive through Rclone before reading its cached OAuth token below.
    // Rclone refreshes expired access tokens and persists the refreshed token in
    // BOREAL's config; the exact Google API lookup can then safely reuse it.
    let drives = inventory::discover_shared_drives(runtime, executable)?;
    let folder = identity::fetch_google_drive_folder(runtime, folder_id)?;
    if folder.drive_id.is_empty() {
        if require_shared_drive {
            return Err("My Drive migrations require a Shared Drive destination folder".into());
        }
        let folders = destination_folder_chain(runtime, folder, "")?;
        let destination = folders
            .last()
            .ok_or("Google Drive returned no destination folder")?;
        return Ok(SharedDriveDestination {
            drive_id: String::new(),
            drive_name: "My Drive".to_string(),
            folder_id: destination.id.clone(),
            folder_name: destination.name.clone(),
            folders,
        });
    }
    if drives.is_empty() {
        return Err("The authenticated read-only account cannot access any Shared Drives".into());
    }
    let drive = drives
        .into_iter()
        .find(|drive| drive.id == folder.drive_id)
        .ok_or("The destination belongs to a Shared Drive that is not available to the authenticated read-only account")?;
    let destination_folder_id = folder.id.clone();
    let folder_name = if folder.id == drive.id {
        drive.name.clone()
    } else {
        folder.name.clone()
    };
    let folders = destination_folder_chain(runtime, folder, &drive.id)?;
    Ok(SharedDriveDestination {
        drive_id: drive.id,
        drive_name: drive.name,
        folder_id: destination_folder_id,
        folder_name,
        folders,
    })
}

fn destination_folder_chain(
    runtime: &Runtime,
    destination: identity::GoogleDriveFolder,
    shared_drive_id: &str,
) -> Result<Vec<identity::GoogleDriveFolder>, RcloneError> {
    let mut folders = vec![destination];
    for _ in 0..100 {
        let Some(parent_id) = folders.last().and_then(|folder| folder.parents.first()) else {
            break;
        };
        if !shared_drive_id.is_empty() && parent_id == shared_drive_id {
            break;
        }
        let parent = identity::fetch_google_drive_folder(runtime, parent_id)?;
        if parent.drive_id != folders[0].drive_id {
            return Err("Destination folder ancestry crosses Google Drive boundaries".into());
        }
        let is_my_drive_root = shared_drive_id.is_empty() && parent.parents.is_empty();
        if !is_my_drive_root {
            folders.push(parent);
        }
        if is_my_drive_root {
            break;
        }
    }
    folders.reverse();
    Ok(folders)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DestinationEntry {
    name: String,
}

pub fn preflight_copy(
    runtime: &Runtime,
    executable: &Path,
    destination_drive_id: &str,
    destination_folder_id: &str,
    sources: &[MigrationSource],
    allow_existing: bool,
) -> Result<(), RcloneError> {
    let config_path = config::path(runtime)?;
    let refresh = command::run(
        executable,
        [
            "backend",
            "drives",
            &format!("{}:", RemoteKind::MyDriveRw.name()),
            "--json",
            "--config",
            config_path.to_string_lossy().as_ref(),
        ],
    )?;
    if !refresh.status.success() {
        return Err(format!(
            "My Drive RW authorization failed: {}",
            String::from_utf8_lossy(&refresh.stderr).trim()
        )
        .into());
    }
    let folder = identity::fetch_google_drive_folder_for_remote(
        runtime,
        RemoteKind::MyDriveRw,
        destination_folder_id,
    )?;
    if folder.drive_id != destination_drive_id {
        return Err("The destination now resolves to a different Shared Drive".into());
    }
    if !folder.can_add_children {
        return Err("The My Drive RW account cannot add content to the destination folder".into());
    }

    let destination = destination_remote(destination_drive_id, destination_folder_id);
    let output = command::run(
        executable,
        [
            "lsjson",
            &destination,
            "--max-depth",
            "1",
            "--config",
            config_path.to_string_lossy().as_ref(),
        ],
    )?;
    if !output.status.success() {
        return Err(format!(
            "Unable to inspect the migration destination: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    let entries: Vec<DestinationEntry> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Unable to parse destination contents: {error}"))?;
    let existing = entries
        .into_iter()
        .map(|entry| entry.name)
        .collect::<HashSet<_>>();
    let mut selected = HashSet::new();
    for source in sources {
        if !selected.insert(source.name.clone()) {
            return Err(format!(
                "Multiple selected items are named '{}'. Select unique top-level names before starting the migration.",
                source.name
            )
            .into());
        }
        if !allow_existing && existing.contains(&source.name) {
            return Err(format!(
                "The destination already contains an item named '{}'. BOREAL will not merge or overwrite it.",
                source.name
            )
            .into());
        }
    }
    Ok(())
}

pub fn copy_source(
    runtime: &Runtime,
    executable: &Path,
    source_kind: &str,
    source_scope: &str,
    source: &MigrationSource,
    destination_drive_id: &str,
    destination_folder_id: &str,
    immutable: bool,
) -> Result<(), RcloneError> {
    let config_path = config::path(runtime)?;
    let source_remote = source_remote(source_kind, source_scope, &source.relative_path)?;
    let destination_root = destination_remote(destination_drive_id, destination_folder_id);
    let destination = format!("{destination_root}{}", source.name);
    let operation = if source.is_directory {
        "copy"
    } else {
        "copyto"
    };
    let mut arguments = vec![
        operation.to_string(),
        source_remote,
        destination,
        "--drive-server-side-across-configs".to_string(),
        "--config".to_string(),
        config_path.to_string_lossy().into_owned(),
    ];
    if immutable {
        arguments.push("--immutable".to_string());
    }
    if source.is_directory {
        arguments.push("--create-empty-src-dirs".to_string());
    }
    let output = command::run(executable, &arguments)?;
    if !output.status.success() {
        return Err(format!(
            "Rclone could not copy '{}': {}",
            source.name,
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(())
}

fn source_remote(kind: &str, scope: &str, path: &str) -> Result<String, RcloneError> {
    let options = if kind == "shared-drive" {
        let id = scope
            .strip_prefix(crate::database::inventory::SHARED_DRIVE_SCOPE_PREFIX)
            .filter(|id| {
                !id.is_empty()
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
            .ok_or("Invalid Shared Drive source")?;
        format!(",team_drive={id}")
    } else if kind == "shared-with-me" {
        ",shared_with_me=true".into()
    } else {
        String::new()
    };
    Ok(format!(
        "{}{}:{}",
        RemoteKind::MyDriveRo.name(),
        options,
        path.trim_start_matches('/')
    ))
}

fn destination_remote(drive_id: &str, folder_id: &str) -> String {
    if drive_id.is_empty() {
        format!(
            "{},root_folder_id={folder_id}:",
            RemoteKind::MyDriveRw.name()
        )
    } else {
        format!(
            "{},team_drive={drive_id},root_folder_id={folder_id}:",
            RemoteKind::MyDriveRw.name()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_drive_copy_uses_source_drive_and_independent_destination() {
        assert_eq!(
            source_remote("shared-drive", "shared-drive:source_1", "Research/Folder").unwrap(),
            "my-drive-ro,team_drive=source_1:Research/Folder"
        );
        assert_eq!(
            destination_remote("target_2", "folder_3"),
            "my-drive-rw,team_drive=target_2,root_folder_id=folder_3:"
        );
        assert_eq!(
            destination_remote("", "personal_folder"),
            "my-drive-rw,root_folder_id=personal_folder:"
        );
        assert!(source_remote("shared-drive", "shared-drive:bad,token=value", "Folder").is_err());
        assert_eq!(
            source_remote("shared-with-me", "shared-with-me", "Folder").unwrap(),
            "my-drive-ro,shared_with_me=true:Folder"
        );
    }
}
