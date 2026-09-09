use std::{collections::HashMap, fs, path::PathBuf};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::bootstrap::Runtime;

pub type GitHubError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub resource_owner: String,
    pub authenticated_login: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expires_on: String,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct ConnectionSummary {
    pub resource_owner: String,
    pub authenticated_login: String,
    pub expires_on: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Owner {
    pub id: i64,
    pub login: String,
    pub html_url: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Repository {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub html_url: String,
    #[serde(default)]
    pub description: Option<String>,
    pub owner: Owner,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub visibility: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub fork: bool,
    #[serde(default)]
    pub is_template: bool,
    #[serde(default)]
    pub default_branch: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub open_issues_count: u64,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub pushed_at: String,
    #[serde(default)]
    pub permissions: RepositoryPermissions,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepositoryPermissions {
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub maintain: bool,
    #[serde(default)]
    pub push: bool,
    #[serde(default)]
    pub triage: bool,
    #[serde(default)]
    pub pull: bool,
}

impl Repository {
    pub fn effective_permission(&self) -> &'static str {
        if self.permissions.admin {
            "admin"
        } else if self.permissions.maintain {
            "maintain"
        } else if self.permissions.push {
            "write"
        } else if self.permissions.triage {
            "triage"
        } else if self.permissions.pull {
            "read"
        } else {
            "unknown"
        }
    }
}

fn credentials_path(runtime: &Runtime) -> Result<PathBuf, GitHubError> {
    Ok(runtime
        .directories
        .get("CONF")
        .ok_or("BOREAL CONF directory is not configured")?
        .join("github-token.json"))
}

pub fn configured(runtime: &Runtime) -> bool {
    load_credentials(runtime).is_ok_and(|credentials| !credentials.connections().is_empty())
}

pub fn load_credentials(runtime: &Runtime) -> Result<Credentials, GitHubError> {
    let contents = fs::read_to_string(credentials_path(runtime)?)?;
    let credentials: Credentials = serde_json::from_str(&contents)?;
    if credentials.connections().is_empty() {
        return Err("GitHub token is empty".into());
    }
    Ok(credentials)
}

impl Credentials {
    fn connections(&self) -> Vec<Connection> {
        let mut connections = self.connections.clone();
        if !self.token.trim().is_empty() {
            connections.push(Connection {
                resource_owner: "Legacy connection".to_string(),
                authenticated_login: String::new(),
                expires_on: String::new(),
                token: self.token.trim().to_string(),
            });
        }
        connections
    }
}

pub fn connection_summaries(runtime: &Runtime) -> Vec<ConnectionSummary> {
    load_credentials(runtime)
        .map(|credentials| {
            credentials
                .connections()
                .into_iter()
                .map(|connection| ConnectionSummary {
                    resource_owner: connection.resource_owner,
                    authenticated_login: connection.authenticated_login,
                    expires_on: connection.expires_on,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn write_credentials(runtime: &Runtime, connections: Vec<Connection>) -> Result<(), GitHubError> {
    let path = credentials_path(runtime)?;
    fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&Credentials {
                token: String::new(),
                connections,
            })?
        ),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn save_connection(
    runtime: &Runtime,
    resource_owner: &str,
    expires_on: &str,
    token: &str,
) -> Result<Account, GitHubError> {
    if token.trim().is_empty() {
        return Err("Enter a GitHub access token".into());
    }
    if resource_owner.trim().is_empty() {
        return Err("Enter the Resource owner selected on GitHub".into());
    }
    let account = account_with_token(token.trim())?;
    // This also verifies the repository Metadata: read permission before storing the secret.
    repositories_with_token(token.trim())?;
    let credentials_file = credentials_path(runtime)?;
    let mut connections = if credentials_file.exists() {
        load_credentials(runtime)?.connections()
    } else {
        Vec::new()
    };
    connections.retain(|connection| {
        !connection
            .resource_owner
            .eq_ignore_ascii_case(resource_owner.trim())
    });
    connections.push(Connection {
        resource_owner: resource_owner.trim().to_string(),
        authenticated_login: account.login.clone(),
        expires_on: expires_on.trim().to_string(),
        token: token.trim().to_string(),
    });
    write_credentials(runtime, connections)?;
    Ok(account)
}

pub fn delete_connection(runtime: &Runtime, resource_owner: &str) -> Result<(), GitHubError> {
    let credentials = load_credentials(runtime)?;
    let mut connections = credentials.connections();
    let original_len = connections.len();
    connections.retain(|connection| {
        !connection
            .resource_owner
            .eq_ignore_ascii_case(resource_owner.trim())
    });
    if connections.len() == original_len {
        return Err("GitHub connection was not found".into());
    }
    write_credentials(runtime, connections)
}

pub fn account_with_token(token: &str) -> Result<Account, GitHubError> {
    let client = Client::builder().user_agent("BOREAL").build()?;
    Ok(client
        .get("https://api.github.com/user")
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()?
        .error_for_status()?
        .json()?)
}

pub fn repositories(runtime: &Runtime) -> Result<Vec<Repository>, GitHubError> {
    let connections = load_credentials(runtime)?.connections();
    let mut repositories_by_id = HashMap::new();
    for connection in connections {
        for repository in repositories_with_token(&connection.token).map_err(|error| {
            format!(
                "GitHub connection for '{}' failed: {error}",
                connection.resource_owner
            )
        })? {
            repositories_by_id.insert(repository.id, repository);
        }
    }
    let mut repositories: Vec<_> = repositories_by_id.into_values().collect();
    repositories.sort_by(|left, right| left.full_name.cmp(&right.full_name));
    Ok(repositories)
}

fn repositories_with_token(token: &str) -> Result<Vec<Repository>, GitHubError> {
    let client = Client::builder().user_agent("BOREAL").build()?;
    let mut repositories = Vec::new();
    for page in 1..=100_u32 {
        let batch: Vec<Repository> = client
            .get("https://api.github.com/user/repos")
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .query(&[
                ("per_page", "100".to_string()),
                ("page", page.to_string()),
                ("visibility", "all".to_string()),
                (
                    "affiliation",
                    "owner,collaborator,organization_member".to_string(),
                ),
            ])
            .send()?
            .error_for_status()?
            .json()?;
        let complete = batch.len() < 100;
        repositories.extend(batch);
        if complete {
            return Ok(repositories);
        }
    }
    Err("GitHub repository listing exceeded 10,000 repositories".into())
}

#[cfg(test)]
mod tests {
    use super::{Connection, Credentials};

    #[test]
    fn reads_legacy_single_token_credentials() {
        let credentials: Credentials =
            serde_json::from_str(r#"{"token":"legacy-secret"}"#).unwrap();
        let connections = credentials.connections();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].resource_owner, "Legacy connection");
        assert_eq!(connections[0].token, "legacy-secret");
    }

    #[test]
    fn reads_multiple_resource_owner_connections() {
        let credentials = Credentials {
            token: String::new(),
            connections: vec![
                Connection {
                    resource_owner: "organization-one".into(),
                    authenticated_login: "user".into(),
                    expires_on: "2026-12-01".into(),
                    token: "one".into(),
                },
                Connection {
                    resource_owner: "organization-two".into(),
                    authenticated_login: "user".into(),
                    expires_on: String::new(),
                    token: "two".into(),
                },
            ],
        };
        assert_eq!(credentials.connections().len(), 2);
    }
}
