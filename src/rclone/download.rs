use std::{path::Path, process::Command};

use crate::rclone::{RcloneError, remotes::RemoteKind};

pub struct DownloadRequest<'a> {
    pub executable: &'a Path,
    pub config_path: &'a Path,
    pub relative_path: &'a str,
    pub destination: &'a Path,
    pub is_directory: bool,
    pub shared_with_me: bool,
    pub shared_drive_id: Option<&'a str>,
    pub immutable: bool,
}

pub fn safe_local_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => character,
        })
        .collect::<String>();
    let sanitized = sanitized.trim().trim_matches('.');
    let reserved = sanitized
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if sanitized.is_empty() {
        "Drive item".to_string()
    } else if matches!(
        reserved.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        format!("_{sanitized}")
    } else {
        sanitized.to_string()
    }
}

pub fn copy_item(request: DownloadRequest<'_>) -> Result<(), RcloneError> {
    let source = format!("{}:{}", RemoteKind::MyDriveRo.name(), request.relative_path);
    let mut command = Command::new(request.executable);
    command.arg(if request.is_directory {
        "copy"
    } else {
        "copyto"
    });
    command.args([
        source.as_str(),
        request.destination.to_string_lossy().as_ref(),
    ]);
    command.args(["--config", request.config_path.to_string_lossy().as_ref()]);
    command.arg("--create-empty-src-dirs");
    if request.immutable {
        command.arg("--immutable");
    }
    if request.shared_with_me {
        command.arg("--drive-shared-with-me");
    }
    if let Some(drive_id) = request.shared_drive_id {
        command.args(["--drive-team-drive", drive_id, "--drive-root-folder-id", ""]);
    }

    let output = command
        .output()
        .map_err(|error| format!("Unable to start Rclone download: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Rclone download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::safe_local_name;

    #[test]
    fn makes_cross_platform_local_names() {
        assert_eq!(
            safe_local_name("Budget: 2026/Final?.xlsx"),
            "Budget_ 2026_Final_.xlsx"
        );
        assert_eq!(safe_local_name(".."), "Drive item");
        assert_eq!(safe_local_name("CON.txt"), "_CON.txt");
    }
}
