//! Apps Script response validation. No credentials, conversations or arbitrary fields are imported.
use super::{
    GoogleError, auth,
    groups::{Group, Member, Snapshot},
};
use crate::bootstrap::Runtime;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Deserialize)]
struct Page {
    schema: u32,
    account: String,
    groups: Vec<ReportGroup>,
    next: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct ReportGroup {
    email: String,
    members: Vec<ReportMember>,
    members_unavailable: bool,
    self_role: String,
}
#[derive(Deserialize)]
struct ReportMember {
    email: String,
    role: String,
    #[serde(rename = "type")]
    kind: String,
}
fn id(email: &str) -> String {
    format!(
        "apps-{:x}",
        Sha256::digest(email.to_ascii_lowercase().as_bytes())
    )
}
fn valid_email(email: &str) -> bool {
    email.len() <= 320
        && email.contains('@')
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
}
fn role(role: &str) -> bool {
    matches!(
        role,
        "OWNER" | "MANAGER" | "MEMBER" | "INVITED" | "PENDING" | "BANNED" | "UNKNOWN"
    )
}
fn parse(
    value: serde_json::Value,
    account: &str,
) -> Result<(Vec<Group>, Option<serde_json::Value>), GoogleError> {
    if value.get("error").is_some() {
        return Err("The My Groups helper could not finish. Check member visibility, Apps Script quotas and the deployed helper version, then retry. Previous inventory was retained.".into());
    }
    let page: Page = serde_json::from_value(
        value
            .get("response")
            .and_then(|v| v.get("result"))
            .cloned()
            .ok_or("The Groups helper did not return a report; check its deployment and version")?,
    )
    .map_err(|_| "Invalid Groups helper report; previous inventory retained")?;
    if page.schema != 1 || !page.account.eq_ignore_ascii_case(account) {
        return Err(
            "Groups helper account or version mismatch; previous inventory retained".into(),
        );
    }
    let mut groups = Vec::new();
    for g in page.groups {
        if !valid_email(&g.email)
            || !role(&g.self_role)
            || (g.members_unavailable && !g.members.is_empty())
        {
            return Err("Invalid Groups helper metadata".into());
        }
        let mut seen = HashSet::new();
        let mut members = Vec::new();
        for m in g.members {
            if !valid_email(&m.email)
                || !role(&m.role)
                || !matches!(m.kind.as_str(), "USER" | "GROUP")
                || !seen.insert(m.email.to_ascii_lowercase())
            {
                return Err("Invalid or duplicate Groups helper member".into());
            }
            members.push(Member {
                id: id(&m.email),
                email: m.email,
                status: match m.role.as_str() {
                    "PENDING" | "INVITED" => "PENDING",
                    "BANNED" => "BANNED",
                    "UNKNOWN" => "UNKNOWN",
                    _ => "ACTIVE",
                }
                .into(),
                role: m.role,
                kind: m.kind,
            });
        }
        groups.push(Group {
            id: id(&g.email),
            email: g.email,
            description: format!(
                "Your membership: {}. Member results may be incomplete.",
                g.self_role.to_lowercase()
            ),
            members,
            members_unavailable: g.members_unavailable,
            ..Default::default()
        });
    }
    Ok((groups, page.next))
}
pub fn snapshot(
    runtime: &Runtime,
    deployment: &str,
    account: &str,
    cancel: &AtomicBool,
) -> Result<Snapshot, GoogleError> {
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(380))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let endpoint = format!("https://script.googleapis.com/v1/scripts/{deployment}:run");
    let mut result = Snapshot {
        account: account.into(),
        groups: vec![],
    };
    let mut cursor = serde_json::Value::Null;
    let mut cursors = HashSet::new();
    let mut identities = HashSet::new();
    let mut total_bytes = 0usize;
    for _ in 0..1000 {
        if cancel.load(Ordering::Relaxed) {
            return Err("Google Groups update canceled".into());
        }
        let token = auth::access_token(runtime, auth::GROUPS)?;
        let response=http.post(&endpoint).bearer_auth(token)
            .json(&serde_json::json!({"function":"borealMyGroups","parameters":[cursor],"devMode":false})).send()
            .map_err(|_| "My Groups request failed or timed out. Retry; previous inventory was retained.")?;
        if !response.status().is_success() {
            return Err(match response.status().as_u16() {
            401=>"Google authorization expired. Reconnect in Settings → Google connection.",
            403=>"Google denied access to the My Groups helper. Check that Apps Script API is enabled, the executable allows this user, and its Cloud project matches Boreal's OAuth client. Your organization may need to allow the app; a Workspace Groups Reader role is not required for this mode.",
            404=>"My Groups helper deployment was not found. Check its deployment ID in Google connection setup.",
            429=>"My Groups quota reached. Retry later; previous inventory was retained.",
            _=>"The My Groups service is temporarily unavailable. Retry the update."
        }.into());
        }
        let mut bytes = Vec::new();
        response
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Unable to read Groups helper report")?;
        total_bytes = total_bytes.saturating_add(bytes.len());
        if total_bytes > 64 * 1024 * 1024 || bytes.len() > 8 * 1024 * 1024 {
            return Err("Groups helper report is too large; previous inventory retained".into());
        }
        let (groups, next) = parse(
            serde_json::from_slice(&bytes).map_err(|_| "Invalid Groups helper response")?,
            account,
        )?;
        for group in groups {
            if !identities.insert(group.id.clone()) {
                return Err(
                    "Groups changed during update; retry. Previous inventory retained.".into(),
                );
            }
            result.groups.push(group);
        }
        let Some(next) = next else {
            return Ok(result);
        };
        if !cursors.insert(next.to_string()) {
            return Err("Groups helper repeated a page; previous inventory retained".into());
        }
        cursor = next;
    }
    Err("Groups helper exceeded its page limit; previous inventory retained".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_reports_are_account_bound_and_allowlisted() {
        let report = serde_json::json!({"response":{"result":{"schema":1,"account":"me@example.test","next":null,"groups":[{"email":"team@example.test","self_role":"PENDING","members_unavailable":false,"members":[{"email":"user@example.test","role":"BANNED","type":"USER","secret":"never import"}]}]}}});
        let (groups, _) = parse(report.clone(), "me@example.test").unwrap();
        assert_eq!(groups[0].members[0].status, "BANNED");
        assert!(groups[0].direct_members_count.is_empty());
        assert!(
            !serde_json::to_string(&groups)
                .unwrap()
                .contains("never import")
        );
        assert!(parse(report, "other@example.test").is_err());
        assert!(
            parse(serde_json::json!({"error":{"message":"secret"}}), "me")
                .err()
                .unwrap()
                .to_string()
                .find("secret")
                .is_none()
        );
    }
}
