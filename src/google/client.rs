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
    let profile: serde_json::Value =
        serde_json::from_slice(data).map_err(|_| "Invalid Google project profile")?;
    let shared_setup = profile
        .get("boreal_google")
        .map(|value| {
            serde_json::from_value::<super::auth::Setup>(value.clone())
                .map_err(|_| "Invalid Boreal Google profile options")
        })
        .transpose()?;
    if let Some(setup) = &shared_setup {
        setup.validate()?;
    }
    let config_path = path(runtime)?;

    let parent = config_path
        .parent()
        .ok_or("Unable to determine Google client configuration directory")?;

    fs::create_dir_all(parent)?;

    super::auth::private_write(&config_path, data)?;
    if let Some(mut setup) = shared_setup {
        // The profile supplies deployment information; users choose write access locally.
        setup.migration_access = super::auth::setup(runtime)
            .unwrap_or_default()
            .migration_access;
        super::auth::save_setup(runtime, &setup)?;
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reusable_profile_imports_helper_without_replacing_user_authorization() {
        let runtime = crate::google::auth::tests::fixture();
        let conf = crate::google::auth::conf(&runtime).unwrap();
        let account = fs::read(conf.join("google-account.json")).unwrap();
        let mut profile: serde_json::Value =
            serde_json::from_slice(&fs::read(path(&runtime).unwrap()).unwrap()).unwrap();
        profile["boreal_google"] = serde_json::json!({"groups_deployment":"deployment_123","directory_admin":false,"migration_access":true});
        import(&runtime, profile.to_string().as_bytes()).unwrap();
        let setup = crate::google::auth::setup(&runtime).unwrap();
        assert_eq!(setup.groups_deployment, "deployment_123");
        assert!(!setup.migration_access);
        assert_eq!(fs::read(conf.join("google-account.json")).unwrap(), account);
        let before = fs::read(path(&runtime).unwrap()).unwrap();
        profile["boreal_google"]["groups_deployment"] = "https://wrong.invalid".into();
        assert!(import(&runtime, profile.to_string().as_bytes()).is_err());
        assert_eq!(fs::read(path(&runtime).unwrap()).unwrap(), before);
        fs::remove_dir_all(&runtime.boreal_home).unwrap();
    }
}
