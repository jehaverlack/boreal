use super::*;

#[derive(Default, serde::Deserialize)]
pub(super) struct GroupsQuery {
    #[serde(default)]
    group: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    sort: String,
    #[serde(default)]
    direction: String,
    #[serde(default)]
    connected: bool,
    #[serde(default)]
    error: String,
}
struct MemberView {
    member: google::groups::Member,
    identity: IdentityDisplay,
}
#[derive(Template)]
#[template(path = "google-groups.html", config = "askama.toml")]
struct GroupsTemplate {
    title: &'static str,
    active_page: &'static str,
    alerts: Vec<AlertItem>,
    status_items: Vec<StatusItem>,
    poll_rclone: bool,
    account: String,
    configured: bool,
    connection_issue: String,
    enabled: bool,
    summary: database::google_groups::Summary,
    query: GroupsQuery,
    groups: Vec<google::groups::Group>,
    members: Vec<MemberView>,
    current_name: String,
    members_unavailable: bool,
}
fn identity_map(
    database: &database::Database,
) -> Result<HashMap<String, Vec<database::inventory::Tag>>, database::DatabaseError> {
    let principals = database::directory::list_principals(database)?;
    let all_tags = database::inventory::list_tags_for_scope(
        database,
        database::inventory::TagScope::Directory,
    )?;
    let c = database.connect()?;
    let mut aliases=c.prepare("SELECT email FROM principal_emails WHERE principal_id=?1 UNION SELECT primary_email FROM principals WHERE id=?1 AND primary_email IS NOT NULL")?;
    let mut result = HashMap::new();
    for p in principals {
        let tags = all_tags
            .iter()
            .filter(|t| p.tags.iter().any(|pt| pt.slug == t.slug))
            .cloned()
            .collect::<Vec<_>>();
        for email in aliases.query_map([p.id], |r| r.get::<_, String>(0))? {
            result.insert(email?.trim().to_lowercase(), tags.clone());
        }
    }
    Ok(result)
}
pub(super) async fn page(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GroupsQuery>,
) -> Result<Html<String>, StatusCode> {
    let account = google::groups::connected_email(&state.runtime).unwrap_or_default();
    let db = state
        .database()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let summary = database::google_groups::summary(&db, &account)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut groups = database::google_groups::list(&db, &account)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let identities = identity_map(&db).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut members = Vec::new();
    let mut current_name = String::new();
    let mut members_unavailable = false;
    let term = query.q.trim().to_lowercase();
    if !query.group.is_empty() {
        let group = groups
            .iter()
            .find(|g| g.id == query.group)
            .ok_or(StatusCode::NOT_FOUND)?;
        current_name = if group.name.is_empty() {
            group.email.clone()
        } else {
            group.name.clone()
        };
        members_unavailable = group.members_unavailable;
        members = group
            .members
            .iter()
            .filter(|m| {
                (term.is_empty()
                    || m.email.to_lowercase().contains(&term)
                    || m.id.to_lowercase().contains(&term))
                    && (query.role.is_empty() || m.role.eq_ignore_ascii_case(&query.role))
            })
            .map(|m| {
                let label = if m.email.is_empty() {
                    m.id.clone()
                } else {
                    m.email.clone()
                };
                let tags = identities.get(&m.email.trim().to_lowercase());
                MemberView {
                    member: m.clone(),
                    identity: identity_display(
                        label,
                        tags.is_some(),
                        tags.map(Vec::as_slice).unwrap_or(&[]),
                    ),
                }
            })
            .collect();
        members.sort_by(|a, b| {
            let key = |m: &MemberView| match query.sort.as_str() {
                "role" => m.member.role.to_lowercase(),
                "type" => m.member.kind.to_lowercase(),
                "status" => m.member.status.to_lowercase(),
                _ => m.identity.label.to_lowercase(),
            };
            let order = key(a)
                .cmp(&key(b))
                .then_with(|| a.member.id.cmp(&b.member.id));
            if query.direction == "desc" {
                order.reverse()
            } else {
                order
            }
        });
    } else {
        groups.retain(|g| {
            (term.is_empty()
                || g.name.to_lowercase().contains(&term)
                || g.email.to_lowercase().contains(&term)
                || g.description.to_lowercase().contains(&term)
                || g.aliases.iter().any(|a| a.to_lowercase().contains(&term))
                || g.members
                    .iter()
                    .any(|m| m.email.to_lowercase().contains(&term)))
                && (query.role.is_empty()
                    || g.members
                        .iter()
                        .any(|m| m.role.eq_ignore_ascii_case(&query.role)))
        });
        groups.sort_by(|a, b| {
            let order = match query.sort.as_str() {
                "email" => a.email.to_lowercase().cmp(&b.email.to_lowercase()),
                "members" => a
                    .direct_members_count
                    .parse::<u64>()
                    .unwrap_or(0)
                    .cmp(&b.direct_members_count.parse::<u64>().unwrap_or(0)),
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            }
            .then_with(|| a.id.cmp(&b.id));
            if query.direction == "desc" {
                order.reverse()
            } else {
                order
            }
        });
    }
    let rclone_state = state.rclone_state();
    let google_client_state = state.google_client_state();
    let remotes = state.google_remotes_state();
    let metadata = state.metadata_state();
    render_template(&GroupsTemplate {
        title: "Google Groups - BOREAL",
        active_page: "google-groups",
        alerts: build_alerts(
            &rclone_state,
            &google_client_state,
            bookmark_reminder_visible(&state),
        ),
        status_items: build_status_items(
            &state,
            &rclone_state,
            &google_client_state,
            &remotes,
            &metadata,
            configured_remote_count(&state.runtime, &rclone_state),
            authenticated_google_email(&state),
            &state.update_state(),
        ),
        poll_rclone: should_poll_ui(&rclone_state, &remotes, &metadata),
        configured: matches!(google_client_state, GoogleClientState::Ready(_)),
        connection_issue: google::groups::connection_issue(&state.runtime)
            .unwrap_or_default()
            .into(),
        enabled: google_groups_enabled(&state),
        account,
        summary,
        groups,
        members,
        current_name,
        members_unavailable,
        query,
    })
}
pub(super) async fn connect(State(state): State<Arc<AppState>>) -> Result<Redirect, StatusCode> {
    if !google_groups_enabled(&state) {
        return Ok(Redirect::to("/settings#google-groups-settings"));
    }
    let result = tokio::task::spawn_blocking(move || {
        google::groups::connect(&state.runtime, || *state.shutdown_receiver().borrow())
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(match result {
        Ok(()) => Redirect::to("/google-groups?connected=true"),
        Err(error) => Redirect::to(&format!(
            "/google-groups?error={}",
            encode_query_value(&error.to_string())
        )),
    })
}
pub(super) fn summary(state: &AppState) -> database::google_groups::Summary {
    google::groups::connected_email(&state.runtime)
        .and_then(|account| {
            state
                .database()
                .ok()
                .and_then(|db| database::google_groups::summary(&db, &account).ok())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn groups_view_renders_setup_metadata_and_restricted_members_safely() {
        let mut template = GroupsTemplate {
            title: "Groups",
            active_page: "google-groups",
            alerts: vec![],
            status_items: vec![],
            poll_rclone: false,
            account: String::new(),
            configured: true,
            connection_issue: String::new(),
            enabled: true,
            summary: Default::default(),
            query: Default::default(),
            groups: vec![],
            members: vec![],
            current_name: String::new(),
            members_unavailable: false,
        };
        let setup = template.render().unwrap();
        assert!(setup.contains("Connect Google Groups"));
        assert!(setup.contains("Admin SDK API"));
        template.account = "me@example.test".into();
        template.connection_issue = "Google Client ID changed. Reconnect Google Groups.".into();
        let stale = template.render().unwrap();
        assert!(stale.contains("Google Client ID changed"));
        assert!(stale.contains("Reconnect / switch account"));
        assert!(!stale.contains(">Update metadata</button>"));
        template.connection_issue.clear();
        template.groups.push(google::groups::Group {
            id: "g".into(),
            name: "<script>alert(1)</script>".into(),
            email: "g@example.test".into(),
            members_unavailable: true,
            ..Default::default()
        });
        let groups = template.render().unwrap();
        assert!(!groups.contains("<script>alert(1)</script>"));
        assert!(groups.contains("Restricted / unavailable"));
        assert!(groups.contains("data-column-widths=\"google-groups\""));
        template.query.group = "g".into();
        template.current_name = "Team".into();
        template.members_unavailable = true;
        template.members.push(MemberView {
            member: google::groups::Member {
                role: "OWNER".into(),
                ..Default::default()
            },
            identity: identity_display("known@example.test".into(), true, &[]),
        });
        let members = template.render().unwrap();
        assert!(members.contains("This does not mean the group is empty"));
        assert!(members.contains("known@example.test"));
        assert!(members.contains("OWNER"));
        assert!(members.contains("data-column-widths=\"google-group-members\""));
    }
}
