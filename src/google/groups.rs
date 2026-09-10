//! Metadata-only Google Groups. My Groups uses the user-scoped Apps Script helper.
use super::{GoogleError, auth};
use crate::bootstrap::Runtime;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
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

pub fn connected_email(runtime: &Runtime) -> Option<String> {
    auth::email(runtime).or_else(|| {
        // Read only the legacy identity to keep cached inventory browsable.
        let bytes =
            std::fs::read(auth::conf(runtime).ok()?.join("google-groups-token.json")).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        value.get("email")?.as_str().map(str::to_string)
    })
}
pub fn connection_issue(runtime: &Runtime) -> Option<&'static str> {
    let Ok(setup) = auth::setup(runtime) else {
        return Some("Review Google connection setup in Settings.");
    };
    if setup.directory_admin {
        auth::issue(runtime, &[auth::DIRECTORY_GROUPS, auth::DIRECTORY_MEMBERS])
    } else if setup.groups_deployment.is_empty() {
        Some(
            "My Groups needs the shared Groups helper. Open Settings → Google connection to configure its deployment once. Workspace admin privileges are not needed for My Groups.",
        )
    } else {
        auth::issue(runtime, &[auth::GROUPS])
    }
}
pub fn connection_ready(runtime: &Runtime) -> bool {
    connection_issue(runtime).is_none()
}
pub fn snapshot(runtime: &Runtime, cancel: &AtomicBool) -> Result<Snapshot, GoogleError> {
    if let Some(issue) = connection_issue(runtime) {
        return Err(issue.into());
    }
    let setup = auth::setup(runtime)?;
    let account = auth::email(runtime).ok_or("Connect Google in Settings")?;
    if setup.directory_admin {
        let token = auth::access_token(runtime, auth::DIRECTORY_GROUPS)?;
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        fetch_snapshot(
            &http,
            &token,
            &account,
            "https://admin.googleapis.com/admin/directory/v1",
            cancel,
        )
    } else {
        super::my_groups::snapshot(runtime, &setup.groups_deployment, &account, cancel)
    }
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
    use std::{io::Write, net::TcpListener};
    fn http() -> Result<reqwest::blocking::Client, GoogleError> {
        Ok(reqwest::blocking::Client::new())
    }
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
