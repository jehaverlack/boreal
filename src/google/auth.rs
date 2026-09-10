//! One private Google grant shared by Drive, private Persons Sheets and Groups.
use super::{GoogleError, client};
use crate::{bootstrap::Runtime, database::settings::InventorySettings};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const DRIVE_READ: &str = "https://www.googleapis.com/auth/drive.readonly";
pub const DRIVE_WRITE: &str = "https://www.googleapis.com/auth/drive";
pub const GROUPS: &str = "https://www.googleapis.com/auth/groups";
pub const DIRECTORY_GROUPS: &str = "https://www.googleapis.com/auth/admin.directory.group.readonly";
pub const DIRECTORY_MEMBERS: &str =
    "https://www.googleapis.com/auth/admin.directory.group.member.readonly";
static CREDENTIAL_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Setup {
    pub groups_deployment: String,
    pub directory_admin: bool,
    pub migration_access: bool,
}
impl Setup {
    pub fn validate(&self) -> Result<(), GoogleError> {
        if self.groups_deployment.len() > 256
            || !self
                .groups_deployment
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err("Enter the Apps Script deployment ID, not a URL.".into());
        }
        Ok(())
    }
}
#[derive(Deserialize, Serialize)]
struct Credentials {
    client_id: String,
    subject: String,
    email: String,
    refresh_token: String,
    access_token: String,
    expires_at: u64,
    scopes: Vec<String>,
}
#[derive(Deserialize)]
struct Token {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    scope: String,
    expires_in: u64,
}
pub fn conf(runtime: &Runtime) -> Result<&Path, GoogleError> {
    runtime
        .directories
        .get("CONF")
        .map(PathBuf::as_path)
        .ok_or_else(|| "Google configuration directory is unavailable".into())
}
pub fn setup(runtime: &Runtime) -> Result<Setup, GoogleError> {
    setup_at(conf(runtime)?)
}
pub(crate) fn setup_at(conf: &Path) -> Result<Setup, GoogleError> {
    let path = conf.join("google-connection.json");
    if !path.exists() {
        return Ok(Setup::default());
    }
    let value: Setup = serde_json::from_slice(&fs::read(path)?)
        .map_err(|_| "Google connection setup could not be read")?;
    value.validate()?;
    Ok(value)
}
pub fn save_setup(runtime: &Runtime, value: &Setup) -> Result<(), GoogleError> {
    value.validate()?;
    private_write(
        &conf(runtime)?.join("google-connection.json"),
        &serde_json::to_vec(value)?,
    )
}
pub fn requested_scopes(settings: &InventorySettings, setup: &Setup) -> Vec<String> {
    let mut scopes = vec!["openid".into(), "email".into()];
    if setup.migration_access {
        scopes.push(DRIVE_WRITE.into());
    } else if settings.google_drive_enabled || settings.directory_sheet_enabled {
        scopes.push(DRIVE_READ.into());
    }
    if settings.google_groups_enabled {
        if setup.directory_admin {
            scopes.extend([DIRECTORY_GROUPS.into(), DIRECTORY_MEMBERS.into()]);
        } else {
            scopes.push(GROUPS.into());
        }
    }
    scopes
}
fn credentials_at(conf: &Path) -> Result<Credentials, GoogleError> {
    serde_json::from_slice(
        &fs::read(conf.join("google-account.json"))
            .map_err(|_| "Connect your Google account in Settings → Google connection")?,
    )
    .map_err(|_| "Google connection could not be read; reconnect in Settings".into())
}
pub(super) fn account_key_at(conf: &Path) -> Result<String, GoogleError> {
    let c = credentials_at(conf)?;
    Ok(format!("{}:{}", c.client_id, c.subject))
}
pub fn configured(runtime: &Runtime) -> bool {
    conf(runtime).is_ok_and(|c| c.join("google-account.json").exists())
}
pub fn email(runtime: &Runtime) -> Option<String> {
    credentials_at(conf(runtime).ok()?).ok().map(|c| c.email)
}
pub fn granted(runtime: &Runtime, scope: &str) -> bool {
    issue(runtime, &[scope]).is_none()
}
fn covers(scopes: &[String], required: &str) -> bool {
    scopes
        .iter()
        .any(|s| s == required || (required == DRIVE_READ && s == DRIVE_WRITE))
}
pub fn issue(runtime: &Runtime, scopes: &[&str]) -> Option<&'static str> {
    let Ok(saved) = conf(runtime).and_then(credentials_at) else {
        return Some(
            "Connect Google once in Settings → Google connection for your enabled services.",
        );
    };
    let Ok(Some(config)) = client::detect(runtime) else {
        return Some("Configure the Google project in Settings → Google connection.");
    };
    credential_issue(&saved, &config.client_id, scopes)
}
fn credential_issue(saved: &Credentials, client: &str, scopes: &[&str]) -> Option<&'static str> {
    if saved.client_id != client {
        Some("Google app changed. Reconnect once in Settings → Google connection.")
    } else if saved.refresh_token.is_empty() {
        Some("Google offline access is missing. Reconnect in Settings → Google connection.")
    } else if !scopes.iter().all(|s| covers(&saved.scopes, s)) {
        Some(
            "Additional Google permission is needed. Connect in Settings → Google connection and approve the selected services.",
        )
    } else {
        None
    }
}
pub fn access_token(runtime: &Runtime, scope: &str) -> Result<String, GoogleError> {
    Ok(access_at(conf(runtime)?, scope)?.0)
}
pub(crate) fn access_at(conf: &Path, scope: &str) -> Result<(String, u64), GoogleError> {
    access_at_endpoint(conf, scope, "https://oauth2.googleapis.com/token")
}
fn access_at_endpoint(
    conf: &Path,
    scope: &str,
    endpoint: &str,
) -> Result<(String, u64), GoogleError> {
    let _guard = CREDENTIAL_LOCK
        .lock()
        .map_err(|_| "Google connection is busy")?;
    let mut saved = credentials_at(conf)?;
    let config = client::validate(
        &fs::read(conf.join("google-client.json"))
            .map_err(|_| "Google project configuration is missing")?,
    )?;
    if let Some(issue) = credential_issue(&saved, &config.client_id, &[scope]) {
        return Err(issue.into());
    }
    if saved.access_token.is_empty() || saved.expires_at <= now().saturating_add(390) {
        let response = http()?
            .post(endpoint)
            .form(&[
                ("client_id", config.client_id.as_str()),
                ("client_secret", config.client_secret.as_str()),
                ("refresh_token", saved.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .map_err(|_| "Unable to reach Google. Retry the update.")?;
        if !response.status().is_success() {
            return Err(if response.status().as_u16() == 400 || response.status().as_u16() == 401 {
                "Google authorization expired or was revoked. Reconnect in Settings → Google connection."
            } else { "Google authorization is temporarily unavailable. Retry the update." }.into());
        }
        let token: Token = response
            .json()
            .map_err(|_| "Invalid Google authorization response")?;
        apply_refresh(&mut saved, token)?;
        private_write(
            &conf.join("google-account.json"),
            &serde_json::to_vec(&saved)?,
        )?;
    }
    if !covers(&saved.scopes, scope) {
        return Err(
            "Google no longer grants this permission. Reconnect in Settings → Google connection."
                .into(),
        );
    }
    Ok((saved.access_token, saved.expires_at.saturating_sub(now())))
}
fn apply_refresh(saved: &mut Credentials, token: Token) -> Result<(), GoogleError> {
    if token.access_token.is_empty() || token.expires_in == 0 {
        return Err("Invalid Google authorization response".into());
    }
    saved.access_token = token.access_token;
    saved.expires_at = now().saturating_add(token.expires_in);
    if !token.refresh_token.is_empty() {
        saved.refresh_token = token.refresh_token;
    }
    if !token.scope.is_empty() {
        saved.scopes = token.scope.split_whitespace().map(str::to_string).collect();
    }
    Ok(())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn save_credentials(runtime: &Runtime, value: &Credentials) -> Result<(), GoogleError> {
    private_write(
        &conf(runtime)?.join("google-account.json"),
        &serde_json::to_vec(value)?,
    )
}
pub(super) fn private_write(path: &Path, bytes: &[u8]) -> Result<(), GoogleError> {
    let parent = path.parent().ok_or("Invalid Google configuration path")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".google-{}.tmp", random_secret()?));
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
    result.map_err(|_| "Unable to save the Google connection privately".into())
}
fn http() -> Result<reqwest::blocking::Client, GoogleError> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
fn random_secret() -> Result<String, GoogleError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Unable to create secure Google sign-in state")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn callback_code(target: &str, expected_state: &str) -> Result<String, GoogleError> {
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "Invalid Google sign-in callback")?;
    let pairs = url.query_pairs().collect::<Vec<_>>();
    if url.path() != "/"
        || pairs.iter().filter(|(k, _)| k == "state").count() != 1
        || !pairs
            .iter()
            .any(|(k, v)| k == "state" && v == expected_state)
    {
        return Err("Google sign-in state did not match".into());
    }
    if pairs.iter().any(|(k, _)| k == "error") {
        return Err("Google sign-in was canceled or denied".into());
    }
    let codes = pairs
        .iter()
        .filter(|(k, v)| k == "code" && !v.is_empty())
        .collect::<Vec<_>>();
    if codes.len() != 1 {
        return Err("Google did not return a sign-in code".into());
    }
    Ok(codes[0].1.to_string())
}
pub fn connect(
    runtime: &Runtime,
    scopes: &[String],
    canceled: impl Fn() -> bool,
) -> Result<(), GoogleError> {
    let _guard = CREDENTIAL_LOCK
        .try_lock()
        .map_err(|_| "Google is already connecting or updating")?;
    let config = client::detect(runtime)?.ok_or("Configure your Google Desktop Client ID first")?;
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|_| "Unable to start Google sign-in callback")?;
    listener.set_nonblocking(true)?;
    let redirect = format!("http://127.0.0.1:{}/", listener.local_addr()?.port());
    let state = random_secret()?;
    let verifier = random_secret()?;
    let mut url = reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("response_type", "code"),
        ("scope", &scopes.join(" ")),
        ("state", state.as_str()),
        ("code_challenge", challenge(&verifier).as_str()),
        ("code_challenge_method", "S256"),
        ("access_type", "offline"),
        ("prompt", "consent select_account"),
    ]);
    webbrowser::open(url.as_str()).map_err(|_| "Unable to open Google sign-in in your browser")?;
    let deadline = Instant::now() + Duration::from_secs(180);
    let code = loop {
        if canceled() {
            return Err("Google sign-in canceled during shutdown".into());
        }
        if Instant::now() > deadline {
            return Err("Google sign-in timed out; try connecting again".into());
        }
        let (mut stream, _) = match listener.accept() {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(_) => return Err("Google sign-in callback failed".into()),
        };
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while request.len() < 8192 {
            if stream.read(&mut byte).unwrap_or(0) == 0 {
                break;
            }
            request.push(byte[0]);
            if request.ends_with(b"\r\n") {
                break;
            }
        }
        let line = String::from_utf8_lossy(&request);
        let mut parts = line.split_whitespace();
        if parts.next() != Some("GET") {
            continue;
        }
        let target = parts.next().unwrap_or("");
        // Unrelated requests must not terminate the pending login.
        if !target.starts_with("/?") {
            continue;
        }
        let result = callback_code(target, &state);
        let message = if result.is_ok() {
            "Google sign-in received. Return to Boreal to finish connecting."
        } else {
            "Sign-in callback was not accepted. Return to Boreal and try again."
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
            message.len(),
            message
        );
        break result?;
    };
    let http = http()?;
    let response = http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .map_err(|_| "Google token exchange could not connect")?;
    if !response.status().is_success() {
        return Err("Google sign-in could not be completed; reconnect".into());
    }
    let token: Token = response
        .json()
        .map_err(|_| "Invalid Google sign-in response")?;
    if token.refresh_token.is_empty() || token.access_token.is_empty() {
        return Err(
            "Google did not grant offline access. Connect again and approve access.".into(),
        );
    }
    #[derive(Deserialize)]
    struct Identity {
        sub: String,
        email: String,
        email_verified: bool,
    }
    let response = http
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(&token.access_token)
        .send()
        .map_err(|_| "Unable to verify connected Google account")?;
    if !response.status().is_success() {
        return Err("Unable to verify connected Google account".into());
    }
    let identity: Identity = response
        .json()
        .map_err(|_| "Invalid Google account identity response")?;
    if !identity.email_verified || identity.sub.is_empty() || identity.email.is_empty() {
        return Err("Google account email is not verified".into());
    }
    if canceled() {
        return Err("Google connection canceled; previous authorization retained".into());
    }
    save_credentials(
        runtime,
        &Credentials {
            client_id: config.client_id,
            subject: identity.sub,
            email: identity.email,
            refresh_token: token.refresh_token,
            access_token: token.access_token,
            expires_at: now().saturating_add(token.expires_in),
            scopes: token.scope.split_whitespace().map(str::to_string).collect(),
        },
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub fn fixture() -> Runtime {
        let dir =
            std::env::temp_dir().join(format!("boreal-google-test-{}", random_secret().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("google-client.json"),serde_json::json!({"installed":{"client_id":"test.apps.googleusercontent.com","client_secret":"synthetic-client","auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token"}}).to_string()).unwrap();
        let runtime = Runtime {
            boreal_home: dir.clone(),
            boreal: serde_json::json!({}),
            directories: std::collections::BTreeMap::from([("CONF".into(), dir)]),
        };
        save_credentials(
            &runtime,
            &Credentials {
                client_id: "test.apps.googleusercontent.com".into(),
                subject: "account-one".into(),
                email: "me@example.test".into(),
                refresh_token: "synthetic-private-refresh".into(),
                access_token: "synthetic-access".into(),
                expires_at: now() + 3600,
                scopes: vec![DRIVE_READ.into(), GROUPS.into()],
            },
        )
        .unwrap();
        runtime
    }
    #[test]
    fn selected_capabilities_share_a_grant_without_admin_scopes() {
        let mut settings = InventorySettings::default();
        settings.directory_sheet_enabled = true;
        settings.google_groups_enabled = true;
        let mut setup = Setup::default();
        let scopes = requested_scopes(&settings, &setup);
        assert!(scopes.contains(&DRIVE_READ.into()));
        assert!(scopes.contains(&GROUPS.into()));
        assert!(!scopes.contains(&DIRECTORY_GROUPS.into()));
        setup.migration_access = true;
        let scopes = requested_scopes(&settings, &setup);
        assert!(scopes.contains(&DRIVE_WRITE.into()));
        assert!(!scopes.contains(&DRIVE_READ.into()));
        setup.directory_admin = true;
        assert!(requested_scopes(&settings, &setup).contains(&DIRECTORY_GROUPS.into()));
        setup.groups_deployment = "https://evil.invalid".into();
        assert!(setup.validate().is_err());
    }
    #[test]
    fn authorization_refresh_retains_identity_and_handles_scope_loss() {
        let runtime = fixture();
        let conf = conf(&runtime).unwrap();
        let mut saved = credentials_at(conf).unwrap();
        assert!(credential_issue(&saved, "new-client", &[DRIVE_READ]).is_some());
        assert!(credential_issue(&saved, &saved.client_id, &[DRIVE_WRITE]).is_some());
        apply_refresh(
            &mut saved,
            Token {
                access_token: "new-access".into(),
                refresh_token: String::new(),
                scope: String::new(),
                expires_in: 3600,
            },
        )
        .unwrap();
        assert_eq!(saved.refresh_token, "synthetic-private-refresh");
        assert_eq!(saved.subject, "account-one");
        assert!(covers(&saved.scopes, GROUPS));
        apply_refresh(
            &mut saved,
            Token {
                access_token: "new-access".into(),
                refresh_token: "rotated".into(),
                scope: GROUPS.into(),
                expires_in: 3600,
            },
        )
        .unwrap();
        assert!(!covers(&saved.scopes, DRIVE_READ));
        assert_eq!(saved.refresh_token, "rotated");
        fs::remove_dir_all(&runtime.boreal_home).unwrap();
    }
    #[test]
    fn expired_access_is_refreshed_once_and_saved_privately() {
        let runtime = fixture();
        let conf = conf(&runtime).unwrap();
        let mut saved = credentials_at(conf).unwrap();
        saved.expires_at = 0;
        save_credentials(&runtime, &saved).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = [0; 8192];
            let _ = stream.read(&mut bytes).unwrap();
            let body =
                r#"{"access_token":"renewed","refresh_token":"rotated-refresh","expires_in":3600}"#;
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        assert_eq!(
            access_at_endpoint(conf, DRIVE_READ, &endpoint).unwrap().0,
            "renewed"
        );
        worker.join().unwrap();
        assert_eq!(
            access_at_endpoint(conf, DRIVE_READ, &endpoint).unwrap().0,
            "renewed",
            "cached token must not contact closed endpoint"
        );
        assert_eq!(
            credentials_at(conf).unwrap().refresh_token,
            "rotated-refresh"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(conf.join("google-account.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(&runtime.boreal_home).unwrap();
    }
    #[test]
    fn pkce_callbacks_reject_wrong_state_and_duplicate_codes() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(callback_code("/?state=s&code=c", "s").unwrap(), "c");
        for url in [
            "/?state=wrong&code=c",
            "/?state=s&code=a&code=b",
            "/?state=s&error=access_denied",
        ] {
            assert!(callback_code(url, "s").is_err());
        }
    }
    #[test]
    fn fresh_and_legacy_setups_never_default_to_directory_queries() {
        let runtime = fixture();
        assert!(!setup(&runtime).unwrap().directory_admin);
        let issue = crate::google::groups::connection_issue(&runtime).unwrap();
        assert!(issue.contains("shared Groups helper"));
        fs::remove_dir_all(&runtime.boreal_home).unwrap();
    }
}
