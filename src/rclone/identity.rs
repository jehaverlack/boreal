use std::{fs, path::Path, process::Command, time::Duration};

use serde_json::Value;

use crate::bootstrap::Runtime;

use super::{RcloneError, config, remotes::RemoteKind};

#[derive(Debug, Clone)]
pub struct RemoteIdentity {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub account_id: Option<String>,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleSheetLocation {
    pub spreadsheet_id: String,
    pub gid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleDriveFolder {
    pub id: String,
    pub name: String,
    pub drive_id: String,
    pub can_add_children: bool,
    pub parents: Vec<String>,
    pub modified_at: String,
}

pub fn fetch_google_drive_folder(
    runtime: &Runtime,
    folder_id: &str,
) -> Result<GoogleDriveFolder, RcloneError> {
    fetch_google_drive_folder_for_remote(runtime, RemoteKind::MyDriveRo, folder_id)
}

pub fn fetch_google_drive_folder_for_remote(
    runtime: &Runtime,
    kind: RemoteKind,
    folder_id: &str,
) -> Result<GoogleDriveFolder, RcloneError> {
    let config_path = config::path(runtime)?;
    let access_token = read_access_token(&config_path, kind)?;
    let mut url = reqwest::Url::parse(&format!(
        "https://www.googleapis.com/drive/v3/files/{folder_id}"
    ))
    .map_err(|error| format!("Unable to build Google Drive folder URL: {error}"))?;
    url.query_pairs_mut()
        .append_pair("supportsAllDrives", "true")
        .append_pair(
            "fields",
            "id,name,mimeType,driveId,parents,modifiedTime,capabilities(canAddChildren)",
        );
    let response = google_client()?
        .get(url)
        .bearer_auth(access_token)
        .send()
        .map_err(|error| format!("Google Drive destination lookup failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("Unable to read Google Drive destination response: {error}"))?;
    if !status.is_success() {
        let message = if status == reqwest::StatusCode::UNAUTHORIZED {
            "Google rejected the refreshed My Drive RO authorization. Reconnect the my-drive-ro remote, then validate the destination again."
                .to_string()
        } else if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::NOT_FOUND
        {
            format!(
                "Google Drive destination lookup returned {status}. Confirm that the authenticated read-only account can open this folder."
            )
        } else {
            format!("Google Drive destination lookup returned {status}")
        };
        return Err(message.into());
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|error| format!("Invalid Google Drive destination response: {error}"))?;
    if value.get("mimeType").and_then(Value::as_str) != Some("application/vnd.google-apps.folder") {
        return Err("The Google Drive link must identify a folder".into());
    }
    let value_string = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Ok(GoogleDriveFolder {
        id: value_string("id").ok_or("Google Drive did not return the destination folder ID")?,
        name: value_string("name").ok_or("Google Drive did not return the destination name")?,
        drive_id: value_string("driveId").unwrap_or_default(),
        can_add_children: value
            .get("capabilities")
            .and_then(|capabilities| capabilities.get("canAddChildren"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        parents: value
            .get("parents")
            .and_then(Value::as_array)
            .map(|parents| {
                parents
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        modified_at: value_string("modifiedTime").unwrap_or_default(),
    })
}

pub fn parse_google_sheet_url(url: &str) -> Result<GoogleSheetLocation, RcloneError> {
    let marker = "docs.google.com/spreadsheets/d/";
    let start = url
        .find(marker)
        .ok_or("Directory source must be a Google Sheets URL")?
        + marker.len();
    let remainder = &url[start..];
    let spreadsheet_id = remainder.split('/').next().unwrap_or("").trim();
    if spreadsheet_id.is_empty()
        || !spreadsheet_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err("Google Sheets URL contains an invalid spreadsheet ID".into());
    }
    let gid = url
        .split(['?', '#', '&'])
        .filter_map(|part| part.strip_prefix("gid="))
        .next()
        .unwrap_or("0");
    if gid.is_empty() || !gid.chars().all(|character| character.is_ascii_digit()) {
        return Err("Google Sheets URL contains an invalid worksheet gid".into());
    }
    Ok(GoogleSheetLocation {
        spreadsheet_id: spreadsheet_id.to_string(),
        gid: gid.to_string(),
    })
}

pub fn download_google_sheet_csv(
    runtime: &Runtime,
    url: &str,
) -> Result<(GoogleSheetLocation, Vec<u8>), RcloneError> {
    let location = parse_google_sheet_url(url)?;
    let config_path = config::path(runtime)?;
    let access_token = read_access_token(&config_path, RemoteKind::MyDriveRo)?;
    let export_url = format!(
        "https://docs.google.com/spreadsheets/d/{}/export?format=csv&gid={}",
        location.spreadsheet_id, location.gid,
    );
    let response = google_client()?
        .get(export_url)
        .bearer_auth(access_token)
        .send()
        .map_err(|error| format!("Unable to download directory spreadsheet: {error}"))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .map_err(|error| format!("Unable to read directory spreadsheet download: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Directory spreadsheet download returned {status}; verify Viewer access and Shared Drive download permissions"
        ).into());
    }
    if bytes.len() > 10 * 1024 * 1024 {
        return Err("Directory spreadsheet CSV is larger than 10 MiB".into());
    }
    Ok((location, bytes.to_vec()))
}

pub fn fetch_shared_drive_permissions(
    runtime: &Runtime,
    drive_id: &str,
) -> Result<Vec<Value>, RcloneError> {
    let config_path = config::path(runtime)?;
    let access_token = read_access_token(&config_path, RemoteKind::MyDriveRo)?;
    let client = google_client()?;
    let mut page_token: Option<String> = None;
    let mut permissions = Vec::new();
    loop {
        let mut url = reqwest::Url::parse(&format!(
            "https://www.googleapis.com/drive/v3/files/{drive_id}/permissions"
        ))
        .map_err(|error| format!("Unable to build Shared Drive permissions URL: {error}"))?;
        url.query_pairs_mut()
            .append_pair("supportsAllDrives", "true")
            .append_pair("pageSize", "100")
            .append_pair(
                "fields",
                "nextPageToken,permissions(id,type,role,emailAddress,displayName,domain)",
            );
        if let Some(token) = page_token.as_deref() {
            url.query_pairs_mut().append_pair("pageToken", token);
        }
        let response = client
            .get(url)
            .bearer_auth(&access_token)
            .send()
            .map_err(|error| format!("Shared Drive permission lookup failed: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| format!("Unable to read Shared Drive permissions: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "Shared Drive permission lookup returned {status} for drive {drive_id}: {body}"
            )
            .into());
        }
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| format!("Invalid Shared Drive permissions response: {error}"))?;
        if let Some(page) = value.get("permissions").and_then(Value::as_array) {
            permissions.extend(page.iter().cloned());
        }
        page_token = value
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(str::to_string);
        if page_token.is_none() {
            break;
        }
    }
    Ok(permissions)
}

pub fn fetch_read_only_account(
    runtime: &Runtime,
    executable: &Path,
) -> Result<RemoteIdentity, RcloneError> {
    let config_path = config::path(runtime)?;
    let output = Command::new(executable)
        .args([
            "config",
            "userinfo",
            &format!("{}:", RemoteKind::MyDriveRo.name()),
            "--json",
            "--timeout",
            "20s",
            "--contimeout",
            "10s",
            "--config",
            config_path.to_string_lossy().as_ref(),
        ])
        .output()
        .map_err(|error| format!("Unable to query authenticated Google account: {error}"))?;
    if output.status.success() {
        return parse_rclone_userinfo(&output.stdout);
    }

    log::debug!(
        "Rclone userinfo unavailable; trying Google Drive about API: {}",
        String::from_utf8_lossy(&output.stderr).trim(),
    );
    fetch_drive_about(&config_path)
}

fn parse_rclone_userinfo(bytes: &[u8]) -> Result<RemoteIdentity, RcloneError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid rclone userinfo response: {error}"))?;
    identity_from_value(value)
}

fn fetch_drive_about(config_path: &Path) -> Result<RemoteIdentity, RcloneError> {
    let access_token = read_access_token(config_path, RemoteKind::MyDriveRo)?;
    let client = google_client()?;
    let response = client
        .get("https://www.googleapis.com/drive/v3/about?fields=user%28displayName%2CemailAddress%2CpermissionId%29")
        .bearer_auth(access_token)
        .send()
        .map_err(|error| format!("Google Drive account lookup failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("Unable to read Google Drive account response: {error}"))?;
    if !status.is_success() {
        return Err(format!("Google Drive account lookup returned {status}: {body}").into());
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|error| format!("Invalid Google Drive account response: {error}"))?;
    let user = value
        .get("user")
        .cloned()
        .ok_or("Google Drive account response did not include a user")?;
    let identity = identity_from_value(user)?;
    if identity.email.is_none() {
        return Err("Google Drive returned user information without an email address".into());
    }
    Ok(identity)
}

fn google_client() -> Result<reqwest::blocking::Client, RcloneError> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("Unable to create Google API client: {error}").into())
}

fn read_access_token(config_path: &Path, kind: RemoteKind) -> Result<String, RcloneError> {
    let config_text = fs::read_to_string(config_path).map_err(|error| {
        format!("Unable to read rclone configuration for Google access: {error}")
    })?;
    let token_json = remote_setting(&config_text, kind.name(), "token").ok_or_else(|| {
        format!(
            "The {} rclone remote does not contain an OAuth token",
            kind.label()
        )
    })?;
    let token: Value = serde_json::from_str(token_json)
        .map_err(|error| format!("Invalid OAuth token in the read-only rclone remote: {error}"))?;
    token
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "The {} rclone remote OAuth token has no access token",
                kind.label()
            )
            .into()
        })
}

fn identity_from_value(value: Value) -> Result<RemoteIdentity, RcloneError> {
    let raw_json = serde_json::to_string(&value)?;
    Ok(RemoteIdentity {
        email: string_field(&value, &["Email", "email", "EmailAddress", "emailAddress"]),
        display_name: string_field(&value, &["Name", "name", "DisplayName", "displayName"]),
        account_id: string_field(
            &value,
            &["ID", "Id", "id", "UserID", "userId", "permissionId"],
        ),
        raw_json,
    })
}

fn remote_setting<'a>(config: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut in_section = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_section = &line[1..line.len() - 1] == section;
            continue;
        }
        if !in_section || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() == key {
            return Some(value.trim());
        }
    }
    None
}

fn string_field(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::{parse_google_sheet_url, remote_setting, string_field};

    #[test]
    fn reads_common_userinfo_field_names() {
        let value = serde_json::json!({
            "Email": "user@example.edu",
            "DisplayName": "Example User",
            "ID": "123"
        });
        assert_eq!(
            string_field(&value, &["email", "Email"]).as_deref(),
            Some("user@example.edu")
        );
        assert_eq!(
            string_field(&value, &["displayName", "DisplayName"]).as_deref(),
            Some("Example User")
        );
    }

    #[test]
    fn reads_token_only_from_requested_remote() {
        let config = "[other]\ntoken = wrong\n[my-drive-ro]\ntype = drive\ntoken = {\"access_token\":\"secret\"}\n";
        assert_eq!(
            remote_setting(config, "my-drive-ro", "token"),
            Some("{\"access_token\":\"secret\"}"),
        );
    }

    #[test]
    fn parses_google_sheet_and_worksheet_ids() {
        let location = parse_google_sheet_url(
            "https://docs.google.com/spreadsheets/d/abc_DEF-123/edit?gid=42#gid=42",
        )
        .expect("Google Sheets URL should parse");
        assert_eq!(location.spreadsheet_id, "abc_DEF-123");
        assert_eq!(location.gid, "42");
    }
}
