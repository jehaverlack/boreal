use std::{fmt, fs, path::PathBuf};

use serde::Deserialize;

use crate::bootstrap::Runtime;

use super::GoogleError;

#[allow(dead_code)]
#[derive(Clone)]
pub struct GoogleClientConfig {
    pub client_id: String,
    pub(crate) client_secret: String,
    pub project_id: Option<String>,
}

impl fmt::Debug for GoogleClientConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleClientConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[redacted]")
            .field("project_id", &self.project_id)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
struct GoogleCredentialsFile {
    installed: GoogleInstalledCredentials,
}

#[derive(Debug, Deserialize)]
struct GoogleInstalledCredentials {
    client_id: String,
    client_secret: String,

    #[serde(default)]
    project_id: Option<String>,

    auth_uri: String,
    token_uri: String,

    #[serde(default)]
    redirect_uris: Vec<String>,
}

/// Return the BOREAL Google OAuth client configuration path.
///
/// Linux/macOS:
///
///     ~/.boreal/conf/google-client.json
///
/// Windows:
///
///     %LOCALAPPDATA%\boreal\conf\google-client.json
pub fn path(runtime: &Runtime) -> Result<PathBuf, GoogleError> {
    let conf_dir = runtime
        .directories
        .get("CONF")
        .ok_or("BOREAL CONF directory is not configured")?;

    Ok(conf_dir.join("google-client.json"))
}

/// Detect and validate an existing Google OAuth client file.
pub fn detect(runtime: &Runtime) -> Result<Option<GoogleClientConfig>, GoogleError> {
    let config_path = path(runtime)?;

    if !config_path.is_file() {
        return Ok(None);
    }

    let data = fs::read(&config_path).map_err(|error| {
        format!(
            "Unable to read Google client configuration {}: {error}",
            config_path.display()
        )
    })?;

    let config = validate(&data)?;

    Ok(Some(config))
}

/// Validate a Google Desktop OAuth credentials JSON file.
pub fn validate(data: &[u8]) -> Result<GoogleClientConfig, GoogleError> {
    let credentials: GoogleCredentialsFile =
        serde_json::from_slice(data).map_err(|_| "Invalid Google Desktop client JSON")?;

    let installed = credentials.installed;

    if installed.client_id.trim().is_empty() {
        return Err("Google client_id is missing".into());
    }

    if !installed.client_id.ends_with(".apps.googleusercontent.com") {
        return Err("Google client_id does not appear to be a valid OAuth Client ID".into());
    }

    if installed.client_secret.trim().is_empty() {
        return Err("Google client_secret is missing".into());
    }

    if installed.auth_uri.trim().is_empty() {
        return Err("Google auth_uri is missing".into());
    }

    if installed.token_uri.trim().is_empty() {
        return Err("Google token_uri is missing".into());
    }

    /*
     * Desktop OAuth clients normally contain redirect URI information.
     * We accept Google's generated values without requiring a specific URI.
     */
    let _ = installed.redirect_uris;

    Ok(GoogleClientConfig {
        client_id: installed.client_id,
        client_secret: installed.client_secret,
        project_id: installed.project_id,
    })
}

/// Validate and save a Google OAuth credentials JSON file.
///
/// The original Google-generated JSON is preserved.
pub fn import(runtime: &Runtime, data: &[u8]) -> Result<GoogleClientConfig, GoogleError> {
    let config = validate(data)?;
    // Import only Desktop application configuration. Retired shared-account/profile
    // fields must never change the existing per-remote Rclone authorization.
    let value: serde_json::Value =
        serde_json::from_slice(data).map_err(|_| "Invalid Google Desktop client JSON")?;
    let data = serde_json::to_vec(&serde_json::json!({"installed": value["installed"]}))?;
    private_write(&path(runtime)?, &data)?;
    Ok(config)
}

fn private_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), GoogleError> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or("Invalid Google client configuration path")?;
    fs::create_dir_all(parent)?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "Unable to prepare private client configuration")?;
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let temp = parent.join(format!(".google-client-{suffix}.tmp"));
    let result = (|| -> Result<(), GoogleError> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result.map_err(|_| "Unable to save Google client configuration privately".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_import_preserves_remote_credentials_and_ignores_retired_profile() {
        let folder = std::env::temp_dir().join(format!(
            "boreal-client-restore-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&folder).unwrap();
        let runtime = Runtime {
            boreal_home: folder.clone(),
            boreal: serde_json::json!({}),
            directories: std::collections::BTreeMap::from([("CONF".into(), folder.clone())]),
        };
        let remote = b"[my-drive-ro]\ntoken = synthetic-ro\n[my-drive-rw]\ntoken = synthetic-rw\n";
        fs::write(folder.join("rclone.conf"), remote).unwrap();
        let input = serde_json::json!({"installed":{"client_id":"test.apps.googleusercontent.com","client_secret":"synthetic-client","auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token"},"boreal_google":{"groups_deployment":"old-helper","directory_admin":true}});
        import(&runtime, input.to_string().as_bytes()).unwrap();
        assert_eq!(fs::read(folder.join("rclone.conf")).unwrap(), remote);
        assert!(!folder.join("google-account.json").exists());
        assert!(!folder.join("google-connection.json").exists());
        let saved = fs::read(path(&runtime).unwrap()).unwrap();
        assert!(
            serde_json::from_slice::<serde_json::Value>(&saved)
                .unwrap()
                .get("boreal_google")
                .is_none()
        );
        assert!(import(&runtime, b"invalid").is_err());
        assert_eq!(fs::read(path(&runtime).unwrap()).unwrap(), saved);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path(&runtime).unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(folder).unwrap();
    }
}
