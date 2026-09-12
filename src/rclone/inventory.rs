use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::Deserialize;

use crate::bootstrap::Runtime;

use super::{RcloneError, config, remotes::RemoteKind};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DriveItem {
    #[serde(default)]
    #[serde(rename = "ID")]
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub mod_time: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SharedDrive {
    pub id: String,
    pub name: String,
    #[serde(default)]
    #[serde(rename = "kind")]
    pub _kind: String,
}

pub fn discover_shared_drives(
    runtime: &Runtime,
    executable: &Path,
) -> Result<Vec<SharedDrive>, RcloneError> {
    if !executable.is_file() {
        return Err(format!("Rclone executable does not exist: {}", executable.display()).into());
    }
    let config_path = config::path(runtime)?;
    let output = Command::new(executable)
        .args([
            "backend",
            "drives",
            &format!("{}:", RemoteKind::MyDriveRo.name()),
            "--json",
            "--config",
            config_path.to_string_lossy().as_ref(),
        ])
        .output()
        .map_err(|error| format!("Unable to discover Shared Drives: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Rclone Shared Drive discovery failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Unable to parse Shared Drive discovery: {error}").into())
}

pub fn fetch_shared_drive(
    runtime: &Runtime,
    executable: &Path,
    scan_id: i64,
    drive_id: &str,
    include_permissions: bool,
) -> Result<Vec<DriveItem>, RcloneError> {
    fetch_drive_with_options(
        runtime,
        executable,
        scan_id,
        include_permissions,
        false,
        Some(drive_id),
    )
}

pub fn fetch_my_drive(
    runtime: &Runtime,
    executable: &Path,
    scan_id: i64,
    include_permissions: bool,
) -> Result<Vec<DriveItem>, RcloneError> {
    fetch_drive_with_options(
        runtime,
        executable,
        scan_id,
        include_permissions,
        false,
        None,
    )
}

pub fn fetch_shared_with_me(
    runtime: &Runtime,
    executable: &Path,
    scan_id: i64,
    include_permissions: bool,
) -> Result<Vec<DriveItem>, RcloneError> {
    fetch_drive_with_options(
        runtime,
        executable,
        scan_id,
        include_permissions,
        true,
        None,
    )
}

pub fn fetch_selected_path(
    runtime: &Runtime,
    executable: &Path,
    inventory_scope: &str,
    relative_path: &str,
    is_directory: bool,
) -> Result<Option<Vec<DriveItem>>, RcloneError> {
    let config_path = config::path(runtime)?;
    let remote = if inventory_scope == crate::database::inventory::SHARED_WITH_ME_SCOPE {
        format!(
            "{},shared_with_me=true:{}",
            RemoteKind::MyDriveRo.name(),
            relative_path.trim_start_matches('/')
        )
    } else if let Some(drive_id) =
        inventory_scope.strip_prefix(crate::database::inventory::SHARED_DRIVE_SCOPE_PREFIX)
    {
        format!(
            "{},team_drive={drive_id}:{}",
            RemoteKind::MyDriveRo.name(),
            relative_path.trim_start_matches('/')
        )
    } else {
        format!(
            "{}:{}",
            RemoteKind::MyDriveRo.name(),
            relative_path.trim_start_matches('/')
        )
    };
    let config_string = config_path.to_string_lossy().into_owned();
    let common = [
        "--metadata",
        "--drive-metadata-owner=read",
        "--drive-metadata-permissions=read",
        "--config",
        config_string.as_str(),
    ];
    let stat = Command::new(executable)
        .args(["lsjson", &remote, "--stat"])
        .args(common)
        .output()
        .map_err(|error| format!("Unable to refresh selected Drive item: {error}"))?;
    if !stat.status.success() {
        let stderr = String::from_utf8_lossy(&stat.stderr);
        if selected_item_not_found(&stderr) {
            return Ok(None);
        }
        return Err(format!("Selected Drive item refresh failed: {}", stderr.trim()).into());
    }
    let mut root: DriveItem = serde_json::from_slice(&stat.stdout)
        .map_err(|error| format!("Unable to parse selected Drive item: {error}"))?;
    root.path = relative_path.to_string();
    let mut items = vec![root];
    if is_directory {
        let output = Command::new(executable)
            .args(["lsjson", &remote, "--recursive"])
            .args(common)
            .output()
            .map_err(|error| format!("Unable to refresh selected Drive folder: {error}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if selected_item_not_found(&stderr) {
                return Ok(None);
            }
            return Err(format!("Selected Drive folder refresh failed: {}", stderr.trim()).into());
        }
        let mut descendants: Vec<DriveItem> = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("Unable to parse selected Drive folder: {error}"))?;
        for item in &mut descendants {
            item.path = format!("{relative_path}/{}", item.path.trim_start_matches('/'));
        }
        items.extend(descendants);
    }
    Ok(Some(items))
}

fn selected_item_not_found(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("directory not found")
        || message.contains("object not found")
        || message.contains("file not found")
}

fn fetch_drive_with_options(
    runtime: &Runtime,
    executable: &Path,
    scan_id: i64,
    include_permissions: bool,
    shared_with_me: bool,
    shared_drive_id: Option<&str>,
) -> Result<Vec<DriveItem>, RcloneError> {
    if !executable.is_file() {
        return Err(format!("Rclone executable does not exist: {}", executable.display()).into());
    }

    let cache_dir = runtime
        .directories
        .get("CACHE")
        .ok_or("BOREAL CACHE directory is not configured")?;
    fs::create_dir_all(cache_dir)?;
    let scope = if shared_with_me {
        "shared-with-me"
    } else if shared_drive_id.is_some() {
        "shared-drive"
    } else {
        "my-drive"
    };
    let cache_path = cache_dir.join(format!("{scope}-inventory-{scan_id}.json"));
    let output_file = File::create(&cache_path)?;
    let config_path = config::path(runtime)?;

    let mut command = Command::new(executable);
    command.args([
        "lsjson",
        &format!("{}:", RemoteKind::MyDriveRo.name()),
        "--recursive",
        "--metadata",
        "--drive-metadata-owner=read",
        "--fast-list",
        "--config",
        config_path.to_string_lossy().as_ref(),
    ]);
    if include_permissions {
        command.arg("--drive-metadata-permissions=read");
    }
    if shared_with_me {
        command.arg("--drive-shared-with-me");
    }
    if let Some(drive_id) = shared_drive_id {
        command.args(["--drive-team-drive", drive_id, "--drive-root-folder-id", ""]);
    }

    let output = command
        .stdout(Stdio::from(output_file))
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("Unable to execute My Drive inventory: {error}"))?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let _ = fs::remove_file(&cache_path);
        return Err(format!("Rclone {scope} inventory failed: {message}").into());
    }

    let result = parse(&cache_path);
    if let Err(error) = fs::remove_file(&cache_path) {
        log::warn!("Unable to remove metadata cache file for scan_id={scan_id}: {error}");
    }
    result
}

fn parse(path: &PathBuf) -> Result<Vec<DriveItem>, RcloneError> {
    let file = File::open(path)?;
    let items: Vec<DriveItem> = serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("Unable to parse Rclone inventory: {error}"))?;

    if let Some(index) = items.iter().position(|item| item.id.trim().is_empty()) {
        return Err(format!(
            "Rclone returned an item without a Drive ID at response index {index}"
        )
        .into());
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::{DriveItem, SharedDrive, selected_item_not_found};

    #[test]
    fn recognizes_only_explicit_not_found_responses() {
        assert!(selected_item_not_found(
            "NOTICE: Failed to lsjson: directory not found"
        ));
        assert!(selected_item_not_found("File not found"));
        assert!(!selected_item_not_found("permission denied"));
        assert!(!selected_item_not_found("connection timed out"));
    }

    #[test]
    fn parses_rclone_uppercase_drive_id() {
        let item: DriveItem = serde_json::from_str(
            r#"{
                "ID":"1AbCdEf",
                "Name":"Report.docx",
                "Path":"2026-08-27/Report.docx",
                "IsDir":false,
                "Size":42,
                "MimeType":"application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "ModTime":"2026-08-27T12:00:00Z",
                "Metadata":{"owner":"owner@example.edu"}
            }"#,
        )
        .expect("rclone item should parse");

        assert_eq!(item.id, "1AbCdEf");
        assert_eq!(item.path, "2026-08-27/Report.docx");
        assert!(!item.is_dir);
    }

    #[test]
    fn parses_shared_drive_discovery() {
        let drives: Vec<SharedDrive> = serde_json::from_str(
            r#"[{"id":"0ABC123","kind":"drive#drive","name":"ACEP Projects"}]"#,
        )
        .expect("Shared Drive discovery should parse");
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].id, "0ABC123");
        assert_eq!(drives[0].name, "ACEP Projects");
    }
}
