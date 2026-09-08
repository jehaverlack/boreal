use serde::Deserialize;
use std::path::Path;

use crate::{bootstrap::Runtime, rclone};

#[derive(Debug, Clone)]
pub struct Object {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub modified_at: String,
    pub is_directory: bool,
    pub mime_type: String,
    pub checksum: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RcloneObject {
    path: String,
    name: String,
    #[serde(default)]
    size: i64,
    #[serde(default)]
    mod_time: String,
    #[serde(default)]
    is_dir: bool,
    #[serde(default)]
    mime_type: String,
    #[serde(default)]
    hashes: std::collections::BTreeMap<String, String>,
}

pub fn inventory(
    runtime: &Runtime,
    executable: &Path,
    remote_name: &str,
) -> Result<Vec<Object>, rclone::RcloneError> {
    let remote_name = validate_remote_name(remote_name)?;
    let config = rclone::config::path(runtime)?;
    let config_text = config.to_string_lossy();
    let target = format!("{remote_name}:");
    let output = rclone::command::run(
        executable,
        [
            "lsjson",
            target.as_str(),
            "--recursive",
            "--fast-list",
            "--hash",
            "--config",
            config_text.as_ref(),
        ],
    )?;
    if !output.status.success() {
        return Err(format!(
            "Unable to inventory S3 remote {remote_name}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    let rows: Vec<RcloneObject> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Invalid S3 inventory returned by Rclone: {error}"))?;
    Ok(rows
        .into_iter()
        .map(|row| Object {
            path: row.path,
            name: row.name,
            size_bytes: row.size.max(0) as u64,
            modified_at: row.mod_time,
            is_directory: row.is_dir,
            mime_type: row.mime_type,
            checksum: row.hashes.into_values().next().unwrap_or_default(),
        })
        .collect())
}

pub fn validate_remote_name(value: &str) -> Result<&str, rclone::RcloneError> {
    let value = value.trim().trim_end_matches(':');
    if value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_. ".contains(character))
    {
        return Err("Enter a valid Rclone remote name".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::validate_remote_name;

    #[test]
    fn validates_rclone_remote_names() {
        assert_eq!(validate_remote_name("research-s3:").unwrap(), "research-s3");
        assert!(validate_remote_name("").is_err());
        assert!(validate_remote_name("s3:bucket").is_err());
    }
}
