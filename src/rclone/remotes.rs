use std::path::Path;

use serde_json::Value;

use crate::{bootstrap::Runtime, google::client::GoogleClientConfig};

use super::{RcloneError, command, config};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteKind {
    MyDriveRw,
    MyDriveRo,
}

impl RemoteKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::MyDriveRw => "my-drive-rw",
            Self::MyDriveRo => "my-drive-ro",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MyDriveRw => "My Drive RW",
            Self::MyDriveRo => "My Drive RO",
        }
    }

    fn scope(self) -> &'static str {
        match self {
            Self::MyDriveRw => "drive",
            Self::MyDriveRo => "drive.readonly",
        }
    }
}

#[derive(Debug, Clone)]
pub enum RemoteState {
    Waiting,
    NotConfigured,
    Configuring,
    Ready,
    Conflict(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredRemote {
    pub name: String,
    pub backend: String,
}

/// List configured remotes without reading or exposing their credentials.
pub fn list_configured(
    runtime: &Runtime,
    executable: &Path,
) -> Result<Vec<ConfiguredRemote>, RcloneError> {
    let config_path = config::path(runtime)?;
    if !config_path.is_file() {
        return Ok(Vec::new());
    }

    let output = command::run(
        executable,
        [
            "listremotes",
            "--long",
            "--config",
            config_path.to_string_lossy().as_ref(),
        ],
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Unable to list Rclone remotes: {}", stderr.trim()).into());
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("Invalid Rclone remote listing: {error}"))?;
    let mut remotes = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let Some((name, backend)) = line.rsplit_once(':') else {
            return Err(format!("Invalid Rclone remote listing row: {line}").into());
        };
        remotes.push(ConfiguredRemote {
            name: name.trim().to_string(),
            backend: backend.trim().to_string(),
        });
    }
    Ok(remotes)
}

enum DetectedRemote {
    Missing,
    NeedsAuthorization,
    Ready,
}

pub fn detect(
    runtime: &Runtime,
    executable: &Path,
    client: &GoogleClientConfig,
    kind: RemoteKind,
) -> RemoteState {
    match inspect(runtime, executable, client, kind) {
        Ok(DetectedRemote::Missing | DetectedRemote::NeedsAuthorization) => {
            RemoteState::NotConfigured
        }
        Ok(DetectedRemote::Ready) => RemoteState::Ready,
        Err(error) => RemoteState::Conflict(error.to_string()),
    }
}

pub fn configure(
    runtime: &Runtime,
    executable: &Path,
    client: &GoogleClientConfig,
    kind: RemoteKind,
) -> Result<(), RcloneError> {
    let config_path = config::path(runtime)?;
    let detected = inspect(runtime, executable, client, kind)?;

    if matches!(detected, DetectedRemote::Ready) {
        return Ok(());
    }

    let output = match detected {
        DetectedRemote::Missing => command::run(
            executable,
            [
                "config",
                "create",
                kind.name(),
                "drive",
                "client_id",
                &client.client_id,
                "client_secret",
                &client.client_secret,
                "scope",
                kind.scope(),
                "--auto-confirm",
                "--obscure",
                "--config",
                config_path.to_string_lossy().as_ref(),
            ],
        )?,
        DetectedRemote::NeedsAuthorization => command::run(
            executable,
            [
                "config",
                "reconnect",
                &format!("{}:", kind.name()),
                "--auto-confirm",
                "--config",
                config_path.to_string_lossy().as_ref(),
            ],
        )?,
        DetectedRemote::Ready => unreachable!(),
    };

    if config_path.is_file() {
        protect_config(&config_path)?;
    }

    if !output.status.success() {
        return Err(authorization_failure(&output).into());
    }

    match inspect(runtime, executable, client, kind)? {
        DetectedRemote::Ready => Ok(()),
        _ => Err(format!("Rclone finished without a usable {} remote", kind.label()).into()),
    }
}

/// Reapply the managed connection settings and authorize again at the user's request.
/// Updates only this remote, preserving every other configured connection.
pub fn reconnect(
    runtime: &Runtime,
    executable: &Path,
    client: &GoogleClientConfig,
    kind: RemoteKind,
) -> Result<(), RcloneError> {
    let config_path = config::path(runtime)?;
    let Some(remote) = read_remote(runtime, executable, kind)? else {
        return configure(runtime, executable, client, kind);
    };
    check_value(&remote, "type", "drive", kind)?;
    let output = command::run(
        executable,
        [
            "config",
            "update",
            kind.name(),
            "client_id",
            &client.client_id,
            "client_secret",
            &client.client_secret,
            "scope",
            kind.scope(),
            "token",
            "",
            "config_refresh_token",
            "false",
            "--obscure",
            "--non-interactive",
            "--config",
            config_path.to_string_lossy().as_ref(),
        ],
    )?;
    protect_config(&config_path)?;
    if !output.status.success() {
        return Err("Could not update this remote. Check that Boreal can write its remote settings, then retry.".into());
    }
    let output = command::run(
        executable,
        [
            "config",
            "reconnect",
            &format!("{}:", kind.name()),
            "--auto-confirm",
            "--config",
            config_path.to_string_lossy().as_ref(),
        ],
    )?;
    protect_config(&config_path)?;
    if !output.status.success() {
        return Err(authorization_failure(&output).into());
    }
    match inspect(runtime, executable, client, kind)? {
        DetectedRemote::Ready => Ok(()),
        _ => Err(
            "Google did not return usable authorization. Reconnect and grant the requested access."
                .into(),
        ),
    }
}

/// Translate known diagnostics to fixed messages. Never echo OAuth output:
/// it can contain tokens, client secrets or authorization URLs.
fn authorization_failure(output: &std::process::Output) -> String {
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    let explanation = if diagnostic.contains("failed to read line")
        || diagnostic.contains("couldn't read answer")
    {
        "Rclone requested terminal input instead of browser sign-in. Restart Boreal with the latest build, then reconnect."
    } else if diagnostic.contains("address already in use")
        || diagnostic.contains("only one usage of each socket address")
    {
        "Google sign-in could not start because its local callback port is already in use. Finish or close any other Rclone sign-in attempt, then reconnect."
    } else if diagnostic.contains("invalid_client") || diagnostic.contains("unauthorized_client") {
        "Google rejected the app credentials. Open Settings → Google setup guide, upload the correct Desktop app credentials, then reconnect."
    } else if diagnostic.contains("access_denied") || diagnostic.contains("access blocked") {
        "Google denied authorization. Check that this account is allowed to use the Google app and grant the requested access when reconnecting."
    } else if diagnostic.contains("connection refused")
        || diagnostic.contains("no such host")
        || diagnostic.contains("timeout")
        || diagnostic.contains("timed out")
    {
        "Google sign-in could not connect or timed out. Check your network connection, then reconnect."
    } else {
        "Google sign-in did not finish. Retry and complete sign-in in the Google tab. If it fails again, check the Google app setup in Settings."
    };
    match output.status.code() {
        Some(code) => format!("{explanation} (Rclone exit code {code}.)"),
        None => format!("{explanation} (Rclone was interrupted.)"),
    }
}

fn inspect(
    runtime: &Runtime,
    executable: &Path,
    client: &GoogleClientConfig,
    kind: RemoteKind,
) -> Result<DetectedRemote, RcloneError> {
    match read_remote(runtime, executable, kind)? {
        Some(remote) => classify_remote(&remote, client, kind),
        None => Ok(DetectedRemote::Missing),
    }
}

fn read_remote(
    runtime: &Runtime,
    executable: &Path,
    kind: RemoteKind,
) -> Result<Option<Value>, RcloneError> {
    let config_path = config::path(runtime)?;
    if !config_path.is_file() {
        return Ok(None);
    }
    let output = command::run(
        executable,
        [
            "config",
            "dump",
            "--config",
            config_path.to_string_lossy().as_ref(),
        ],
    )?;
    if !output.status.success() {
        return Err(
            "Unable to read storage remotes. Check that Boreal can access its Rclone configuration."
                .into(),
        );
    }
    let remotes: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "The saved remote configuration could not be read")?;
    Ok(remotes.get(kind.name()).cloned())
}

fn classify_remote(
    remote: &Value,
    client: &GoogleClientConfig,
    kind: RemoteKind,
) -> Result<DetectedRemote, RcloneError> {
    check_value(remote, "type", "drive", kind)?;
    check_value(remote, "scope", kind.scope(), kind)?;
    check_value(remote, "client_id", &client.client_id, kind)?;

    match remote.get("token").and_then(Value::as_str) {
        Some(token) if !token.trim().is_empty() => Ok(DetectedRemote::Ready),
        _ => Ok(DetectedRemote::NeedsAuthorization),
    }
}

fn check_value(
    remote: &Value,
    key: &str,
    expected: &str,
    kind: RemoteKind,
) -> Result<(), RcloneError> {
    let actual = remote.get(key).and_then(Value::as_str).unwrap_or("");
    if actual == expected {
        Ok(())
    } else {
        let reason = match key {
            "type" => {
                "uses a different storage provider. Rename that remote in the Rclone remote manager, then add the default Google remote again"
            }
            "scope" => {
                "has different Google access permissions. Use Repair / reconnect to apply Boreal's expected permissions"
            }
            "client_id" => {
                "belongs to a different Google app setup. Use Repair / reconnect to authorize the currently configured Google app"
            }
            _ => "needs to be reconfigured",
        };
        Err(format!("Remote '{}' {reason}.", kind.name()).into())
    }
}

#[cfg(unix)]
fn protect_config(path: &Path) -> Result<(), RcloneError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn protect_config(_path: &Path) -> Result<(), RcloneError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn client() -> GoogleClientConfig {
        GoogleClientConfig {
            client_id: "test.apps.googleusercontent.com".to_string(),
            client_secret: "secret".to_string(),
            project_id: None,
        }
    }

    #[test]
    fn conflict_messages_explain_recovery_without_credential_values() {
        for key in ["type", "scope", "client_id"] {
            let mut remote = json!({"type":"drive", "scope":"drive.readonly", "client_id": client().client_id, "token":"private-token"});
            remote[key] = json!("private-value");
            let message = classify_remote(&remote, &client(), RemoteKind::MyDriveRo)
                .err()
                .unwrap()
                .to_string();
            assert!(!message.contains("private-value"));
            assert!(!message.contains("private-token"));
            assert!(message.contains("my-drive-ro"));
            assert!(message.contains("Rename") || message.contains("Repair / reconnect"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn reconnect_updates_only_requested_remote_and_hides_failed_oauth_output() {
        use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};
        let folder = std::env::temp_dir().join(format!(
            "boreal-reconnect-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&folder).unwrap();
        let runtime = Runtime {
            boreal_home: folder.clone(),
            boreal: json!({}),
            directories: BTreeMap::from([("CONF".into(), folder.clone())]),
        };
        fs::write(folder.join("rclone.conf"), "test configuration").unwrap();
        let executable = folder.join("fake-rclone");
        // All credentials in this fixture are synthetic; never invoke a real provider.
        fs::write(&executable, r#"#!/bin/sh
case "$2" in
  dump) printf '%s' '{"my-drive-ro":{"type":"drive","client_id":"old-app","scope":"drive","token":"old-token"}}';;
  update)
    [ "$3" = "my-drive-ro" ] && [ "$4" = "client_id" ] && [ "$8" = "scope" ] && [ "$9" = "drive.readonly" ] && [ "${10}" = "token" ] && [ -z "${11}" ] || exit 7
    ;;
  reconnect) [ "$3" = "my-drive-ro:" ] || exit 8
    case " $* " in *" --auto-confirm "*) ;; *) printf '%s' 'Failed to read line: EOF' >&2; exit 10;; esac
    printf '%s' 'sensitive-oauth-output' >&2
    exit 1;;
  *) exit 9;;
esac
"#).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let message = reconnect(&runtime, &executable, &client(), RemoteKind::MyDriveRo)
            .unwrap_err()
            .to_string();
        assert!(message.contains("sign-in did not finish"), "{message}");
        assert!(!message.contains("sensitive-oauth-output"));
        assert_eq!(
            fs::metadata(folder.join("rclone.conf"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(folder).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn new_and_unfinished_connections_do_not_require_terminal_input() {
        use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};
        let folder = std::env::temp_dir().join(format!(
            "boreal-auth-flags-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&folder).unwrap();
        let runtime = Runtime {
            boreal_home: folder.clone(),
            boreal: json!({}),
            directories: BTreeMap::from([("CONF".into(), folder.clone())]),
        };
        fs::write(folder.join("rclone.conf"), "synthetic fixture").unwrap();
        let executable = folder.join("fake-rclone");
        for remote in [
            json!({}),
            json!({"my-drive-ro": {"type":"drive", "scope":"drive.readonly", "client_id": client().client_id, "token":""}}),
        ] {
            let script = format!(
                r#"#!/bin/sh
if [ "$2" = dump ]; then
    printf '%s' '{remote}'
    exit 0
fi
case " $* " in *" --auto-confirm "*) ;; *) printf '%s' 'Failed to read line: EOF' >&2; exit 10;; esac
printf '%s' 'synthetic-private-output' >&2
exit 1
"#
            );
            fs::write(&executable, script).unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            let message = configure(&runtime, &executable, &client(), RemoteKind::MyDriveRo)
                .unwrap_err()
                .to_string();
            assert!(
                message.contains("Google sign-in did not finish"),
                "{message}"
            );
            assert!(!message.contains("synthetic-private-output"));
        }
        fs::remove_dir_all(folder).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn authorization_errors_are_actionable_and_never_echo_oauth_output() {
        use std::os::unix::process::ExitStatusExt;
        for (diagnostic, expected) in [
            ("Failed to read line: EOF", "terminal input"),
            ("bind: address already in use", "callback port"),
            ("invalid_client", "app credentials"),
            ("access_denied", "denied authorization"),
            ("dial tcp: no such host", "network connection"),
            ("unknown failure", "Google app setup"),
        ] {
            let output = std::process::Output { status: std::process::ExitStatus::from_raw(256), stdout: b"client_secret=private-secret".to_vec(), stderr: format!("{diagnostic} refresh_token=private-token https://example.test/?code=private-code").into_bytes() };
            let message = authorization_failure(&output);
            assert!(message.contains(expected), "{message}");
            assert!(message.contains("exit code 1"));
            for secret in [
                "private-secret",
                "private-token",
                "private-code",
                "https://example.test",
            ] {
                assert!(!message.contains(secret));
            }
        }
    }

    #[test]
    fn uses_expected_remote_names_and_scopes() {
        assert_eq!(RemoteKind::MyDriveRw.name(), "my-drive-rw");
        assert_eq!(RemoteKind::MyDriveRw.scope(), "drive");
        assert_eq!(RemoteKind::MyDriveRo.name(), "my-drive-ro");
        assert_eq!(RemoteKind::MyDriveRo.scope(), "drive.readonly");
    }

    #[test]
    fn recognizes_a_ready_remote() {
        let remote = json!({
            "type": "drive",
            "scope": "drive.readonly",
            "client_id": "test.apps.googleusercontent.com",
            "token": "{\"refresh_token\":\"token\"}"
        });

        assert!(matches!(
            classify_remote(&remote, &client(), RemoteKind::MyDriveRo),
            Ok(DetectedRemote::Ready)
        ));
    }

    #[test]
    fn rejects_a_conflicting_scope() {
        let remote = json!({
            "type": "drive",
            "scope": "drive",
            "client_id": "test.apps.googleusercontent.com",
            "token": "token"
        });

        let error = classify_remote(&remote, &client(), RemoteKind::MyDriveRo)
            .err()
            .expect("scope conflict should fail");
        assert!(
            error
                .to_string()
                .contains("different Google access permissions")
        );
    }
}
