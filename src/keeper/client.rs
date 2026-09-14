use std::{
    collections::HashSet,
    path::PathBuf,
    process::{Command, Stdio},
};

use serde::Deserialize;

use crate::bootstrap::Runtime;

pub type KeeperError = Box<dyn std::error::Error + Send + Sync>;

const SESSION_REQUIRED: &str = "Keeper cannot resume an authenticated session for background indexing. Open Keeper with BOREAL's configured executable and config path, complete SSO/MFA, and run this-device register. If your organization permits it, run this-device persistent-login on. Verify login-status in a separate terminal process using the same config, then retry Save and test access.";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedFolder {
    pub folder_uid: String,
    pub parent_uid: String,
    pub name: String,
    pub folder_type: String,
    pub folder_path: String,
    pub access: Vec<FolderAccess>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderAccess {
    pub shared_to: String,
    pub permissions: String,
    pub target_kind: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultSnapshot {
    pub schema_version: u32,
    pub folders: Vec<SharedFolder>,
    pub records: Vec<Record>,
    pub memberships: Vec<Membership>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub record_uid: String,
    pub title: String,
    pub record_type: String,
    pub modified_ms: i64,
    pub version: u32,
    pub attachment_count: u32,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Membership {
    pub folder_uid: String,
    pub record_uid: String,
}

pub fn config_path(runtime: &Runtime) -> Result<PathBuf, KeeperError> {
    let conf = runtime
        .directories
        .get("CONF")
        .ok_or("BOREAL CONF directory is not configured")?
        .join("keeper");
    std::fs::create_dir_all(&conf)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&conf, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(conf.join("config.json"))
}

pub fn default_command(runtime: &Runtime) -> String {
    let filename = if cfg!(windows) {
        "keeper.exe"
    } else {
        "keeper"
    };
    runtime
        .directories
        .get("BIN")
        .map(|bin| bin.join(filename))
        .filter(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| filename.to_string())
}

pub fn version(command: &str) -> Result<String, KeeperError> {
    let output = Command::new(command_path(command)?)
        .arg("--version")
        .output()?;
    if !output.status.success() {
        return Err(command_error("Keeper Commander version check failed", &output.stderr).into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn vault_snapshot(runtime: &Runtime, command: &str) -> Result<VaultSnapshot, KeeperError> {
    let output = Command::new(commander_python(command)?)
        .arg("-c")
        .arg(include_str!("metadata_report.py"))
        .arg("--config")
        .arg(config_path(runtime)?)
        .stdin(Stdio::null())
        .output()?;
    if output.status.code() == Some(2) {
        return Err(SESSION_REQUIRED.into());
    }
    if !output.status.success() {
        return Err("Keeper metadata report failed. Verify the session and use a Python/pipx installation of Keeper Commander.".into());
    }
    parse_snapshot(&output.stdout)
}

fn parse_snapshot(output: &[u8]) -> Result<VaultSnapshot, KeeperError> {
    // Do not include raw JSON, parser errors or subprocess output in diagnostics.
    let snapshot: VaultSnapshot = serde_json::from_slice(output)
        .map_err(|_| "Keeper returned an invalid metadata-only report")?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn validate_snapshot(snapshot: &VaultSnapshot) -> Result<(), KeeperError> {
    let folders: HashSet<_> = snapshot
        .folders
        .iter()
        .map(|f| f.folder_uid.as_str())
        .collect();
    let records: HashSet<_> = snapshot
        .records
        .iter()
        .map(|r| r.record_uid.as_str())
        .collect();
    let safe_uid = |uid: &str| {
        uid.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    };
    if folders.iter().any(|uid| !safe_uid(uid)) || records.iter().any(|uid| !safe_uid(uid)) {
        return Err("Keeper metadata report contains an invalid identifier".into());
    }
    let located_records: HashSet<_> = snapshot
        .memberships
        .iter()
        .map(|m| m.record_uid.as_str())
        .collect();
    if records != located_records {
        return Err("Keeper metadata report is missing record memberships".into());
    }
    if snapshot.schema_version != 1
        || !folders.contains("")
        || folders.len() != snapshot.folders.len()
        || records.len() != snapshot.records.len()
        || records.contains("")
        || snapshot
            .records
            .iter()
            .any(|r| r.modified_ms < 0 || r.size_bytes < 0)
        || snapshot.memberships.iter().any(|m| {
            !folders.contains(m.folder_uid.as_str()) || !records.contains(m.record_uid.as_str())
        })
    {
        return Err("Keeper metadata report is incomplete or unsupported".into());
    }
    let parents: std::collections::HashMap<_, _> = snapshot
        .folders
        .iter()
        .map(|f| (f.folder_uid.as_str(), f.parent_uid.as_str()))
        .collect();
    if parents.get("") != Some(&"") {
        return Err("Keeper metadata report has an invalid vault root".into());
    }
    for folder in &snapshot.folders {
        let mut current = folder.folder_uid.as_str();
        let mut seen = HashSet::new();
        while !current.is_empty() {
            if !seen.insert(current) {
                return Err("Keeper metadata report contains a folder cycle".into());
            }
            current = parents
                .get(current)
                .copied()
                .ok_or("Keeper metadata report is missing a parent folder")?;
        }
    }
    Ok(())
}

fn commander_python(command: &str) -> Result<PathBuf, KeeperError> {
    let path = command_path(command)?;
    let executable = if path.components().count() > 1 || path.is_absolute() {
        path
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(&path))
            .find(|candidate| candidate.is_file())
            .ok_or("Keeper Commander executable was not found")?
    };
    let executable = executable.canonicalize()?;
    let directory = executable.parent().ok_or("Invalid Keeper Commander path")?;
    for name in ["python", "python3", "python.exe"] {
        let python = directory.join(name);
        if python.is_file() {
            return Ok(python);
        }
    }
    // Console scripts installed outside a virtualenv may name their interpreter.
    let script = std::fs::read_to_string(&executable).unwrap_or_default();
    if let Some(interpreter) = script
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("#!"))
    {
        let interpreter = PathBuf::from(interpreter.trim());
        if interpreter.is_absolute()
            && interpreter.is_file()
            && interpreter
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("python"))
        {
            return Ok(interpreter);
        }
    }
    Err("Vault metadata requires Keeper Commander's Python environment. Configure the keeper executable installed with pip or pipx; standalone bundled executables are not supported.".into())
}

fn command_path(command: &str) -> Result<PathBuf, KeeperError> {
    let command = command.trim();
    if command.is_empty() {
        return Err("Configure the Keeper Commander executable path".into());
    }
    if command == "~" || command.starts_with("~/") || command.starts_with("~\\") {
        let home = dirs::home_dir().ok_or("Unable to determine the user home directory")?;
        let relative = command
            .trim_start_matches('~')
            .trim_start_matches(['/', '\\']);
        return Ok(home.join(relative));
    }
    Ok(PathBuf::from(command))
}

fn command_error(prefix: &str, _stderr: &[u8]) -> String {
    // Commander errors may contain account data; never forward raw diagnostics.
    prefix.to_string()
}

#[cfg(test)]
mod tests {
    use super::{command_path, parse_snapshot};

    #[test]
    fn metadata_protocol_rejects_unknown_fields_and_redacts_errors() {
        let safe = serde_json::json!({"schema_version":1,"folders":[{"folder_uid":"","parent_uid":"","name":"My Vault","folder_type":"Vault","folder_path":"/","access":[]}],"records":[{"record_uid":"one","title":"Title","record_type":"login","modified_ms":0,"version":3,"attachment_count":0,"size_bytes":0}],"memberships":[{"folder_uid":"","record_uid":"one"}]});
        assert!(parse_snapshot(safe.to_string().as_bytes()).is_ok());
        let mut secret = safe.clone();
        secret["records"][0]["password"] = serde_json::json!("SECRET-VALUE");
        let error = parse_snapshot(secret.to_string().as_bytes())
            .unwrap_err()
            .to_string();
        assert!(!error.contains("SECRET"));
        let mut invalid = safe.clone();
        invalid["schema_version"] = serde_json::json!(2);
        assert!(parse_snapshot(invalid.to_string().as_bytes()).is_err());
        let mut invalid = safe.clone();
        invalid["records"][0]["record_uid"] = serde_json::json!("bad&uid");
        assert!(parse_snapshot(invalid.to_string().as_bytes()).is_err());
        assert!(
            !parse_snapshot(b"SECRET-malformed")
                .unwrap_err()
                .to_string()
                .contains("SECRET")
        );
    }

    #[test]
    fn expands_a_user_relative_commander_path() {
        let path = command_path("~/.boreal/keeper-env/bin/keeper").unwrap();
        assert!(path.ends_with(".boreal/keeper-env/bin/keeper"));
        assert!(!path.to_string_lossy().starts_with('~'));
    }
}
