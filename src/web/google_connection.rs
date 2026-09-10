use super::*;
use google::auth;

#[derive(Default, serde::Deserialize)]
pub(super) struct ConnectionForm {
    #[serde(default)]
    drive: Option<String>,
    #[serde(default)]
    groups: Option<String>,
    #[serde(default)]
    sheet_url: String,
    #[serde(default)]
    migration_access: Option<String>,
    #[serde(default)]
    directory_admin: Option<String>,
    #[serde(default)]
    groups_deployment: String,
}
#[derive(Default, serde::Deserialize)]
pub(super) struct ConnectionQuery {
    #[serde(default)]
    notice: String,
    #[serde(default)]
    error: String,
}
#[derive(Template)]
#[template(path = "google-connection.html", config = "askama.toml")]
struct ConnectionTemplate {
    title: &'static str,
    active_page: &'static str,
    alerts: Vec<AlertItem>,
    status_items: Vec<StatusItem>,
    poll_rclone: bool,
    settings: InventorySettings,
    setup: auth::Setup,
    email: String,
    project: String,
    client_ready: bool,
    drive_issue: String,
    groups_issue: String,
    notice: String,
    error: String,
}
pub(super) async fn page(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ConnectionQuery>,
) -> Result<Html<String>, StatusCode> {
    let db = state
        .database()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let settings = settings::load(&db).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (setup, setup_error) = match auth::setup(&state.runtime) {
        Ok(s) => (s, String::new()),
        Err(e) => (Default::default(), e.to_string()),
    };
    let client = google::client::detect(&state.runtime).ok().flatten();
    let rclone = state.rclone_state();
    let remotes = state.google_remotes_state();
    let metadata = state.metadata_state();
    let client_state = state.google_client_state();
    render_template(&ConnectionTemplate {
        title: "Google connection - BOREAL",
        active_page: "settings",
        alerts: build_alerts(&rclone, &client_state, bookmark_reminder_visible(&state)),
        status_items: build_status_items(
            &state,
            &rclone,
            &client_state,
            &remotes,
            &metadata,
            configured_remote_count(&state.runtime, &rclone),
            authenticated_google_email(&state),
            &state.update_state(),
        ),
        poll_rclone: should_poll_ui(&rclone, &remotes, &metadata),
        settings,
        setup,
        email: auth::email(&state.runtime).unwrap_or_default(),
        project: client
            .as_ref()
            .and_then(|c| c.project_id.clone())
            .unwrap_or_default(),
        client_ready: client.is_some(),
        drive_issue: auth::issue(&state.runtime, &[auth::DRIVE_READ])
            .unwrap_or_default()
            .into(),
        groups_issue: google::groups::connection_issue(&state.runtime)
            .unwrap_or_default()
            .into(),
        notice: query.notice,
        error: if query.error.is_empty() {
            setup_error
        } else {
            query.error
        },
    })
}
fn save(state: &AppState, form: &ConnectionForm) -> Result<InventorySettings, String> {
    if !state.active_job_descriptions().is_empty() {
        return Err(
            "Wait for active jobs to finish before changing the Google account or services.".into(),
        );
    }
    let db = state.database()?;
    let mut settings = settings::load(&db).map_err(|e| e.to_string())?;
    let url = form.sheet_url.trim();
    if !url.is_empty() {
        rclone::identity::parse_google_sheet_url(url).map_err(|e| e.to_string())?;
    }
    let setup = auth::Setup {
        groups_deployment: form.groups_deployment.trim().into(),
        directory_admin: form.directory_admin.is_some(),
        migration_access: form.migration_access.is_some(),
    };
    setup.validate().map_err(|e| e.to_string())?;
    settings.google_drive_enabled = form.drive.is_some();
    settings.google_groups_enabled = form.groups.is_some();
    settings.directory_sheet_enabled = !url.is_empty();
    settings.directory_sheet_url = url.into();
    settings.validate().map_err(|e| e.to_string())?;
    auth::save_setup(&state.runtime, &setup).map_err(|e| e.to_string())?;
    settings::save(&db, &settings).map_err(|e| e.to_string())?;
    Ok(settings)
}
fn redirect(result: Result<String, String>) -> Redirect {
    match result {
        Ok(notice) => Redirect::to(&format!("/google?notice={}", encode_query_value(&notice))),
        Err(error) => Redirect::to(&format!("/google?error={}", encode_query_value(&error))),
    }
}
pub(super) async fn configure(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ConnectionForm>,
) -> Redirect {
    redirect(save(&state, &form).map(|_| {
        "Google source choices saved. Connect once to grant any missing permissions.".into()
    }))
}
pub(super) async fn connect(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ConnectionForm>,
) -> Redirect {
    let settings = match save(&state, &form) {
        Ok(s) => s,
        Err(e) => return redirect(Err(e)),
    };
    let result=tokio::task::spawn_blocking(move || -> Result<String,String> {
        let setup=auth::setup(&state.runtime).map_err(|e|e.to_string())?;
        if settings.google_groups_enabled && !setup.directory_admin && setup.groups_deployment.is_empty() {
            return Err("My Groups needs the shared helper deployment ID below. Complete that one-time project step, or deselect Groups to connect your other services now.".into());
        }
        let scopes=auth::requested_scopes(&settings,&setup);
        auth::connect(&state.runtime,&scopes,||*state.shutdown_receiver().borrow() || !state.active_job_descriptions().is_empty()).map_err(|e|e.to_string())?;
        check(&state,&settings)
    }).await;
    redirect(
        result.unwrap_or_else(|_| Err("Google connection task could not finish. Retry.".into())),
    )
}
pub(super) async fn verify(State(state): State<Arc<AppState>>) -> Redirect {
    let result = tokio::task::spawn_blocking(move || {
        let db = state.database()?;
        let settings = settings::load(&db).map_err(|e| e.to_string())?;
        check(&state, &settings)
    })
    .await;
    redirect(result.unwrap_or_else(|_| Err("Google access check could not finish. Retry.".into())))
}
fn check(state: &AppState, settings: &InventorySettings) -> Result<String, String> {
    let mut messages = vec![];
    if settings.google_drive_enabled {
        let result = (|| -> Result<(), google::GoogleError> {
            let token = auth::access_token(&state.runtime, auth::DRIVE_READ)?;
            let response = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()?
                .get("https://www.googleapis.com/drive/v3/files?pageSize=1&fields=files(id)")
                .bearer_auth(token)
                .send()
                .map_err(|_| "Unable to reach Google Drive")?;
            if !response.status().is_success() {
                return Err("Check Drive API enablement and the account's app access".into());
            }
            Ok(())
        })();
        messages.push(match result {
            Ok(()) => "Drive access verified".into(),
            Err(e) => format!("Drive: {e}"),
        });
    }
    if settings.directory_sheet_enabled {
        let result = rclone::identity::download_google_sheet_csv(
            &state.runtime,
            &settings.directory_sheet_url,
        )
        .and_then(|(_, csv)| database::directory::validate_csv(&csv).map(|_| ()));
        messages.push(match result {
            Ok(()) => "Private Persons Sheet access verified".into(),
            Err(e) => format!("Persons Sheet: {e}"),
        });
    }
    if settings.google_groups_enabled {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let result = google::groups::snapshot(&state.runtime, &cancel);
        messages.push(match result {
            Ok(s) => format!("Groups access verified ({} groups)", s.groups.len()),
            Err(e) => format!("Groups: {e}"),
        });
    }
    if auth::setup(&state.runtime).is_ok_and(|s| s.migration_access) {
        messages.push(if auth::granted(&state.runtime, auth::DRIVE_WRITE) {
            "Migration permission granted; destination access is checked before each operation"
                .into()
        } else {
            "Migration permission was not granted; reconnect to approve write access".into()
        });
    }
    Ok(format!(
        "Google connection saved. {}. Use Update metadata for enabled services with access. These checks do not import data.",
        messages.join(". ")
    ))
}
pub(super) async fn helper_code() -> impl axum::response::IntoResponse {
    (
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=Code.gs",
            ),
        ],
        include_str!("../../tools/google-groups/Code.gs"),
    )
}
pub(super) async fn helper_manifest() -> impl axum::response::IntoResponse {
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=appsscript.json",
            ),
        ],
        include_str!("../../tools/google-groups/appsscript.json"),
    )
}

pub(super) async fn profile(
    State(state): State<Arc<AppState>>,
) -> Result<impl axum::response::IntoResponse, StatusCode> {
    let path = google::client::path(&state.runtime).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let bytes = std::fs::read(path).map_err(|_| StatusCode::NOT_FOUND)?;
    let config = google::client::validate(&bytes).map_err(|_| StatusCode::CONFLICT)?;
    let mut setup = auth::setup(&state.runtime).map_err(|_| StatusCode::CONFLICT)?;
    setup.migration_access = false;
    // Construct an allowlist. Never echo extra fields from an uploaded credentials file.
    let profile = serde_json::json!({"installed":{"client_id":config.client_id,"client_secret":config.client_secret,"project_id":config.project_id,"auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token","redirect_uris":["http://localhost"]},"boreal_google":setup});
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=boreal-google-project.json",
            ),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        profile.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn google_connection_renders_one_consent_flow_and_private_sheet_setup() {
        let page = ConnectionTemplate {
            title: "Google connection",
            active_page: "settings",
            alerts: vec![],
            status_items: vec![],
            poll_rclone: false,
            settings: InventorySettings {
                google_groups_enabled: true,
                directory_sheet_url: "https://docs.google.com/spreadsheets/d/example/edit?gid=0"
                    .into(),
                ..Default::default()
            },
            setup: Default::default(),
            email: String::new(),
            project: "Example project".into(),
            client_ready: true,
            drive_issue: String::new(),
            groups_issue: "Configure the shared helper".into(),
            notice: String::new(),
            error: String::new(),
        };
        let html = page.render().unwrap();
        assert_eq!(html.matches("action=\"/google/connect\"").count(), 1);
        assert!(html.contains("Your Sheet can stay private"));
        assert!(html.contains("My Groups helper deployment ID"));
        assert!(!html.contains("id=\"google-admin-mode\" checked"));
        assert!(html.contains("/google/profile"));
        assert!(!html.contains("/remotes/add"));
        if let Ok(directory) = std::env::var("BOREAL_UI_FIXTURE_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                std::path::Path::new(&directory).join("google-connection.html"),
                html,
            )
            .unwrap();
        }
    }
}
