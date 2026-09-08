use std::time::Duration;

use serde::Deserialize;

pub const CHANGELOG_URL: &str =
    "https://raw.githubusercontent.com/jehaverlack/uaf-boreal/main/changelog.json";
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone)]
pub enum UpdateState {
    Checking,
    Current { latest: Release },
    Available { release: Release },
    Error(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub version: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub maturity: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Deserialize)]
struct Changelog {
    releases: Vec<Release>,
}

pub fn check() -> UpdateState {
    match fetch_latest() {
        Ok(latest) if version_is_newer(&latest.version, env!("CARGO_PKG_VERSION")) => {
            UpdateState::Available { release: latest }
        }
        Ok(latest) => UpdateState::Current { latest },
        Err(error) => UpdateState::Error(error),
    }
}

fn fetch_latest() -> Result<Release, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| format!("Unable to configure the update check: {error}"))?;
    let response = client
        .get(CHANGELOG_URL)
        .header(
            reqwest::header::USER_AGENT,
            concat!("BOREAL/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .map_err(|error| format!("Unable to retrieve the BOREAL changelog: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GitHub returned an error for the BOREAL changelog: {error}"))?;
    let changelog: Changelog = response
        .json()
        .map_err(|error| format!("Unable to read the BOREAL changelog: {error}"))?;
    changelog
        .releases
        .into_iter()
        .filter(|release| parse_version(&release.version).is_some())
        .max_by_key(|release| parse_version(&release.version).unwrap_or_default())
        .ok_or_else(|| "The BOREAL changelog contains no valid releases".to_string())
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate)
        .is_some_and(|candidate| parse_version(current).is_some_and(|current| candidate > current))
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let parsed = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(parsed)
}

pub fn release_url(version: &str) -> String {
    format!("https://github.com/jehaverlack/boreal/releases/tag/v{version}")
}

pub fn download_url(version: &str) -> Option<String> {
    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        return None;
    };
    let architecture = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(all(target_os = "linux", target_arch = "arm")) {
        "armv7"
    } else {
        return None;
    };
    let extension = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    let filename = format!("boreal-v{version}-{platform}-{architecture}{extension}");
    Some(format!(
        "https://github.com/jehaverlack/boreal/raw/refs/tags/v{version}/dist/{filename}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_numeric_release_versions() {
        assert!(version_is_newer("1.10.0", "1.9.9"));
        assert!(!version_is_newer("1.0.0", "1.0.0"));
        assert!(!version_is_newer("0.9.9", "1.0.0"));
        assert!(!version_is_newer("invalid", "1.0.0"));
    }
}
