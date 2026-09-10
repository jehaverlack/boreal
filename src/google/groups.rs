//! Read-only Workspace groups for the connected user. Credentials never enter inventory data.
use super::{GoogleError, client};
use crate::bootstrap::Runtime;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const GROUP_SCOPE: &str = "https://www.googleapis.com/auth/admin.directory.group.readonly";
const MEMBER_SCOPE: &str = "https://www.googleapis.com/auth/admin.directory.group.member.readonly";
static CREDENTIAL_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Member {
    pub id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub role: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub status: String,
}
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Group {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "directMembersCount")]
    pub direct_members_count: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default, rename = "nonEditableAliases")]
    pub non_editable_aliases: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub members: Vec<Member>,
    #[serde(skip_deserializing, default)]
    pub members_unavailable: bool,
}
#[derive(Default)]
pub struct Snapshot {
    pub account: String,
    pub groups: Vec<Group>,
}
#[derive(Deserialize)]
struct GroupPage {
    #[serde(default)]
    groups: Vec<Group>,
    #[serde(default, rename = "nextPageToken")]
    next: String,
}
#[derive(Deserialize)]
struct MemberPage {
    #[serde(default)]
    members: Vec<Member>,
    #[serde(default, rename = "nextPageToken")]
    next: String,
}
#[derive(Deserialize, Serialize)]
struct Credentials {
    client_id: String,
    email: String,
    refresh_token: String,
}
#[derive(Deserialize)]
struct Token {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    scope: String,
}

fn token_path(runtime: &Runtime) -> Result<PathBuf, GoogleError> {
    Ok(runtime
        .directories
        .get("CONF")
        .ok_or("BOREAL configuration directory is unavailable")?
        .join("google-groups-token.json"))
}
fn credentials(runtime: &Runtime) -> Result<Credentials, GoogleError> {
    let bytes = fs::read(token_path(runtime)?).map_err(|_| "Connect Google Groups first")?;
    serde_json::from_slice(&bytes)
        .map_err(|_| "Google Groups connection could not be read; reconnect".into())
}
pub fn connected_email(runtime: &Runtime) -> Option<String> {
    credentials(runtime).ok().map(|c| c.email)
}
/// Local readiness only. Keep the saved account identity available for browsing
/// existing inventory, even when its authorization needs to be replaced.
pub fn connection_issue(runtime: &Runtime) -> Option<&'static str> {
    let Ok(saved) = credentials(runtime) else {
        return Some("Connect Google Groups to import your Workspace groups and visible members.");
    };
    let Ok(Some(config)) = client::detect(runtime) else {
        return Some(
            "Configure Google app credentials in Settings → Google setup guide, then reconnect Google Groups.",
        );
    };
    credential_issue(&saved, &config.client_id)
}

fn credential_issue(saved: &Credentials, current_client_id: &str) -> Option<&'static str> {
    if saved.client_id != current_client_id {
        Some(
            "Google Client ID changed. Reconnect Google Groups to authorize the current Google app. Reconnecting Drive does not reconnect Groups.",
        )
    } else if saved.refresh_token.trim().is_empty() {
        Some("Google Groups authorization is missing. Reconnect Google Groups.")
    } else {
        None
    }
}

pub fn connection_ready(runtime: &Runtime) -> bool {
    connection_issue(runtime).is_none()
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
        return Err("Google Groups sign-in was canceled or denied".into());
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
fn save_credentials(runtime: &Runtime, value: &Credentials) -> Result<(), GoogleError> {
    let path = token_path(runtime)?;
    let parent = path.parent().ok_or("Invalid connection directory")?;
    fs::create_dir_all(parent)
        .map_err(|_| "Unable to create Google Groups connection directory")?;
    // Create a fresh private file before replacing the previous connection.
    let temp = parent.join(format!(".google-groups-{}.tmp", random_secret()?));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<(), GoogleError> {
        let mut file = options
            .open(&temp)
            .map_err(|_| "Unable to save Google Groups connection")?;
        file.write_all(&serde_json::to_vec(value)?)
            .map_err(|_| "Unable to save Google Groups connection")?;
        file.sync_all()
            .map_err(|_| "Unable to save Google Groups connection")?;
        fs::rename(&temp, &path).map_err(|_| "Unable to replace Google Groups connection")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn connect(runtime: &Runtime, canceled: impl Fn() -> bool) -> Result<(), GoogleError> {
    let _guard = CREDENTIAL_LOCK
        .try_lock()
        .map_err(|_| "Google Groups is already connecting or updating")?;
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
        (
            "scope",
            &format!("openid email {GROUP_SCOPE} {MEMBER_SCOPE}"),
        ),
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
            return Err("Google Groups sign-in canceled during shutdown".into());
        }
        if Instant::now() > deadline {
            return Err("Google Groups sign-in timed out; try connecting again".into());
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
        .map_err(|_| "Google Groups token exchange could not connect")?;
    if !response.status().is_success() {
        return Err("Google Groups sign-in could not be completed; reconnect".into());
    }
    let token: Token = response
        .json()
        .map_err(|_| "Invalid Google Groups sign-in response")?;
    if token.refresh_token.is_empty()
        || ![GROUP_SCOPE, MEMBER_SCOPE]
            .iter()
            .all(|scope| token.scope.split_whitespace().any(|s| s == *scope))
    {
        return Err("Google Groups needs both read-only group and membership permissions; reconnect and grant both".into());
    }
    #[derive(Deserialize)]
    struct Identity {
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
    if !identity.email_verified {
        return Err("Google account email is not verified".into());
    }
    save_credentials(
        runtime,
        &Credentials {
            client_id: config.client_id,
            email: identity.email,
            refresh_token: token.refresh_token,
        },
    )
}

pub fn snapshot(runtime: &Runtime, cancel: &AtomicBool) -> Result<Snapshot, GoogleError> {
    let _guard = CREDENTIAL_LOCK
        .try_lock()
        .map_err(|_| "Google Groups is already connecting or updating")?;
    let mut credentials = credentials(runtime)?;
    let config = client::detect(runtime)?.ok_or("Configure your Google Desktop Client ID first")?;
    if let Some(issue) = credential_issue(&credentials, &config.client_id) {
        return Err(issue.into());
    }
    let http = http()?;
    let response = http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("refresh_token", credentials.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .map_err(|_| "Unable to connect to Google Groups; try again")?;
    if !response.status().is_success() {
        return Err("Google Groups authorization expired or was revoked; reconnect".into());
    }
    let token: Token = response
        .json()
        .map_err(|_| "Invalid Google Groups authorization response")?;
    if !token.refresh_token.is_empty() {
        credentials.refresh_token = token.refresh_token;
        save_credentials(runtime, &credentials)?;
    }
    fetch_snapshot(
        &http,
        &token.access_token,
        &credentials.email,
        "https://admin.googleapis.com/admin/directory/v1",
        cancel,
    )
}
fn get(
    http: &reqwest::blocking::Client,
    token: &str,
    url: reqwest::Url,
    cancel: &AtomicBool,
) -> Result<reqwest::blocking::Response, GoogleError> {
    for attempt in 0..3 {
        if cancel.load(Ordering::Relaxed) {
            return Err("Google Groups update canceled".into());
        }
        let response = http
            .get(url.clone())
            .bearer_auth(token)
            .send()
            .map_err(|_| "Google Groups request failed; retry the update")?;
        if (response.status().as_u16() == 429 || response.status().is_server_error()) && attempt < 2
        {
            std::thread::sleep(Duration::from_secs(1 << attempt));
            continue;
        }
        return Ok(response);
    }
    unreachable!()
}
fn require_success(
    response: reqwest::blocking::Response,
    operation: &str,
) -> Result<reqwest::blocking::Response, GoogleError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status().as_u16();
    // Inspect a bounded error body only for known categories. Provider messages
    // can contain account details; never copy them into logs or the WebUI.
    let mut body = Vec::new();
    let _ = response.take(65_536).read_to_end(&mut body);
    let body = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    Err(format!(
        "{} ({operation}; HTTP {status}.)",
        api_error_message(status, &body)
    )
    .into())
}

fn api_error_message(status: u16, body: &serde_json::Value) -> &'static str {
    let error = &body["error"];
    let reasons = error["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(error["details"].as_array().into_iter().flatten())
        .filter_map(|item| item["reason"].as_str())
        .collect::<Vec<_>>();
    let has = |reason: &str| reasons.contains(&reason);
    match status {
        401 => "Google Groups authorization expired; reconnect Google Groups in Settings.",
        403 if has("SERVICE_DISABLED") || has("accessNotConfigured") => {
            "Admin SDK API is disabled or has not been enabled for this Google app's project. Enable it in Google Cloud Console using the project associated with Boreal's Google Client ID, then retry the update."
        }
        403 if has("ACCESS_TOKEN_SCOPE_INSUFFICIENT") || has("insufficientPermissions") => {
            "Google Groups authorization is missing required scopes. Reconnect Google Groups in Settings and grant both read-only group and membership permissions."
        }
        403 if has("domainPolicy") || has("ORG_RESTRICTION_VIOLATION") => {
            "Your Workspace policy blocks this Google app. Ask your Workspace administrator to review the app's access under Security → API controls."
        }
        403 if has("rateLimitExceeded") || has("userRateLimitExceeded") || has("quotaExceeded") => {
            "Google Groups rate limit or quota reached; retry later."
        }
        403 if has("forbidden")
            && error["message"].as_str().is_some_and(|message| {
                message
                    .to_ascii_lowercase()
                    .contains("not authorized to access this resource")
            }) =>
        {
            "Google denied Workspace Directory access for the connected account. Ask your Workspace administrator to verify its Admin API privileges → Groups → Read role assignment (or Groups Reader role). Being a group owner or a Google Cloud project administrator alone does not grant this Directory API access. Then retry the update."
        }
        403 => {
            "Google denied this Directory API request. Ask your Workspace administrator to check the connected account's Groups → Read API privilege and the Google app's access policy. This response does not identify a disabled API or missing OAuth scope."
        }
        429 => "Google Groups rate limit reached; retry later.",
        _ => "Google Groups could not be queried; retry the update.",
    }
}

fn fetch_snapshot(
    http: &reqwest::blocking::Client,
    token: &str,
    email: &str,
    base: &str,
    cancel: &AtomicBool,
) -> Result<Snapshot, GoogleError> {
    let mut snapshot = Snapshot {
        account: email.into(),
        groups: vec![],
    };
    let mut page = String::new();
    let mut pages = HashSet::new();
    let mut ids = HashSet::new();
    loop {
        let mut url = reqwest::Url::parse(&format!("{base}/groups"))?;
        url.query_pairs_mut().extend_pairs([("userKey",email),("maxResults","200"),("pageToken",page.as_str()),("fields","nextPageToken,groups(id,email,name,description,directMembersCount,aliases,nonEditableAliases)")]);
        let response = get(http, token, url, cancel)?;
        let response = require_success(response, "groups.list")?;
        let response: GroupPage = response
            .json()
            .map_err(|_| "Invalid Google Groups report; previous inventory was retained")?;
        for group in response.groups {
            if group.id.is_empty() || group.email.is_empty() || !ids.insert(group.id.clone()) {
                return Err("Invalid or duplicate Google Groups identity in report".into());
            }
            snapshot.groups.push(group);
        }
        if response.next.is_empty() {
            break;
        }
        if !pages.insert(response.next.clone()) {
            return Err(
                "Google Groups returned a repeated page; previous inventory was retained".into(),
            );
        }
        page = response.next;
    }
    for group in &mut snapshot.groups {
        page.clear();
        pages.clear();
        let mut members = HashSet::new();
        loop {
            let mut url = reqwest::Url::parse(&format!("{base}/groups/"))?;
            url.path_segments_mut()
                .map_err(|_| "Invalid Google Groups endpoint")?
                .pop_if_empty()
                .push(&group.id)
                .push("members");
            url.query_pairs_mut().extend_pairs([
                ("maxResults", "200"),
                ("includeDerivedMembership", "false"),
                ("pageToken", page.as_str()),
                ("fields", "nextPageToken,members(id,email,role,type,status)"),
            ]);
            let response = get(http, token, url, cancel)?;
            if matches!(response.status().as_u16(), 403 | 404) {
                group.members.clear();
                group.members_unavailable = true;
                break;
            }
            let response = require_success(response, "members.list")?;
            let response: MemberPage = response.json().map_err(
                |_| "Invalid Google Groups member report; previous inventory was retained",
            )?;
            for member in response.members {
                if member.id.is_empty() || !members.insert(member.id.clone()) {
                    return Err("Invalid or duplicate Google Groups member identity".into());
                }
                group.members.push(member);
            }
            if response.next.is_empty() {
                break;
            }
            if !pages.insert(response.next.clone()) {
                return Err("Google Groups returned a repeated membership page".into());
            }
            page = response.next;
        }
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_denials_distinguish_setup_causes_without_disclosing_provider_data() {
        use serde_json::json;
        for (body, expected) in [
            (
                json!({"error":{"errors":[{"reason":"forbidden"}],"message":"Not Authorized to access this resource/api: private-account@example.test"}}),
                "Admin API privileges",
            ),
            (
                json!({"error":{"details":[{"reason":"SERVICE_DISABLED"}],"message":"private-project"}}),
                "Admin SDK API is disabled",
            ),
            (
                json!({"error":{"errors":[{"reason":"accessNotConfigured"}]}}),
                "Admin SDK API is disabled",
            ),
            (
                json!({"error":{"details":[{"reason":"ACCESS_TOKEN_SCOPE_INSUFFICIENT"}]}}),
                "missing required scopes",
            ),
            (
                json!({"error":{"errors":[{"reason":"insufficientPermissions"}]}}),
                "missing required scopes",
            ),
            (
                json!({"error":{"errors":[{"reason":"domainPolicy"}]}}),
                "Workspace policy",
            ),
            (
                json!({"error":{"errors":[{"reason":"rateLimitExceeded"}]}}),
                "rate limit",
            ),
            (
                json!({"error":{"message":"unknown private-provider-message"}}),
                "does not identify",
            ),
            (serde_json::Value::Null, "does not identify"),
        ] {
            let message = api_error_message(403, &body);
            assert!(message.contains(expected), "{message}");
            assert!(!message.contains("private-"));
        }
    }

    #[test]
    fn changed_google_client_requires_groups_reauthorization() {
        let mut saved = Credentials {
            client_id: "old-client".into(),
            email: "person@example.test".into(),
            refresh_token: "synthetic-token".into(),
        };
        assert!(
            credential_issue(&saved, "new-client")
                .unwrap()
                .contains("Reconnect Google Groups")
        );
        assert_eq!(
            saved.email, "person@example.test",
            "keep the account identity for cached inventory"
        );
        assert!(credential_issue(&saved, "old-client").is_none());
        saved.refresh_token.clear();
        assert!(
            credential_issue(&saved, "old-client")
                .unwrap()
                .contains("authorization is missing")
        );
    }

    #[test]
    fn oauth_pkce_and_callback_validate_state() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(
            callback_code("/?state=expected&code=code%2Bvalue", "expected").unwrap(),
            "code+value"
        );
        for callback in [
            "/?state=wrong&code=secret",
            "/?code=secret",
            "/?state=expected&state=other&code=secret",
            "/?state=expected&error=access_denied",
            "/?state=expected&code=a&code=b",
        ] {
            let error = callback_code(callback, "expected").unwrap_err().to_string();
            assert!(!error.contains("secret"));
        }
        let one = random_secret().unwrap();
        assert_eq!(one.len(), 43);
        assert_ne!(one, random_secret().unwrap());
    }
    fn serve(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut requests = vec![];
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = vec![];
                let mut byte = [0u8; 1];
                while !bytes.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    bytes.push(byte[0]);
                }
                requests.push(String::from_utf8(bytes).unwrap());
                write!(stream,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        (base, worker)
    }
    #[test]
    fn paginates_own_groups_and_members_and_marks_restricted_lists() {
        let (base, worker) = serve(vec![
            (
                200,
                r#"{"groups":[{"id":"g1","email":"team@example.test","name":"Team"}],"nextPageToken":"groups-2"}"#,
            ),
            (
                200,
                r#"{"groups":[{"id":"g2","email":"restricted@example.test"}]}"#,
            ),
            (
                200,
                r#"{"members":[{"id":"u1","email":"owner@example.test","role":"OWNER","type":"USER"}],"nextPageToken":"members-2"}"#,
            ),
            (
                200,
                r#"{"members":[{"id":"nested","email":"nested@example.test","role":"MEMBER","type":"GROUP"}]}"#,
            ),
            (403, r#"{"error":{"message":"sensitive provider details"}}"#),
        ]);
        let snapshot = fetch_snapshot(
            &http().unwrap(),
            "test-token",
            "me@example.test",
            &base,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(snapshot.groups.len(), 2);
        assert_eq!(snapshot.groups[0].members.len(), 2);
        assert!(snapshot.groups[1].members_unavailable);
        assert!(snapshot.groups[1].members.is_empty());
        let requests = worker.join().unwrap();
        assert!(requests[0].contains("userKey=me%40example.test"));
        assert!(!requests[0].contains("customer="));
        assert!(requests[1].contains("pageToken=groups-2"));
        assert!(requests[3].contains("pageToken=members-2"));
        assert!(requests[2].contains("includeDerivedMembership=false"));
    }
    #[test]
    fn failed_or_repeated_group_pages_do_not_return_partial_snapshots() {
        let (base, worker) = serve(vec![
            (
                200,
                r#"{"groups":[{"id":"g","email":"g@example.test"}],"nextPageToken":"again"}"#,
            ),
            (200, r#"{"nextPageToken":"again"}"#),
        ]);
        assert!(
            fetch_snapshot(
                &http().unwrap(),
                "secret",
                "me@example.test",
                &base,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        worker.join().unwrap();
        let (base, worker) = serve(vec![(403, r#"{"error":{"message":"secret"}}"#)]);
        let result = fetch_snapshot(
            &http().unwrap(),
            "secret",
            "me@example.test",
            &base,
            &AtomicBool::new(false),
        );
        assert!(!result.err().unwrap().to_string().contains("secret"));
        worker.join().unwrap();
    }
}
