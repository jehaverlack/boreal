use std::{
    env,
    ffi::OsStr,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

use zip::ZipArchive;

use crate::bootstrap::Runtime;

use super::{RcloneError, command, executable_path};

/// Install the current Rclone release into BOREAL's
/// private user-local bin directory.
///
/// No administrator/root permissions are required.
///
/// BOREAL does not:
///
/// - use a system package manager
/// - modify PATH
/// - modify a system Rclone installation
pub fn install<F>(runtime: &Runtime, progress: F) -> Result<PathBuf, RcloneError>
where
    F: FnMut(u64, Option<u64>),
{
    let destination = executable_path(runtime)?;

    let bin_dir = destination
        .parent()
        .ok_or("Unable to determine BOREAL bin directory")?;

    fs::create_dir_all(bin_dir)?;

    let platform = rclone_platform()?;

    let download_url = format!("https://downloads.rclone.org/rclone-current-{platform}.zip");

    let archive_path = temporary_archive_path();

    let extracted_path = temporary_executable_path(bin_dir);

    println!("==> Rclone is not installed");

    println!("==> Installing BOREAL-managed Rclone");

    println!("==> Platform: {platform}");

    println!("==> Downloading: {download_url}");

    remove_if_exists(&archive_path)?;

    remove_if_exists(&extracted_path)?;

    if let Err(error) = download(&download_url, &archive_path, progress) {
        let _ = remove_if_exists(&archive_path);

        let _ = remove_if_exists(&extracted_path);

        return Err(error);
    }

    println!("==> Extracting Rclone");

    if let Err(error) = extract_rclone(&archive_path, &extracted_path) {
        let _ = remove_if_exists(&archive_path);

        let _ = remove_if_exists(&extracted_path);

        return Err(error);
    }

    set_executable_permissions(&extracted_path)?;

    println!("==> Verifying downloaded Rclone");

    let version = command::version(&extracted_path)?;

    println!("==> Downloaded {version}");

    remove_if_exists(&destination)?;

    fs::rename(&extracted_path, &destination).map_err(|error| {
        format!(
            "Unable to install Rclone to {}: {error}",
            destination.display()
        )
    })?;

    remove_if_exists(&archive_path)?;

    println!("==> Rclone installed: {}", destination.display());

    println!("==> {version}");

    Ok(destination)
}

fn rclone_platform() -> Result<&'static str, RcloneError> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    match (os, arch) {
        ("linux", "x86_64") => Ok("linux-amd64"),

        ("linux", "aarch64") => Ok("linux-arm64"),

        ("linux", "arm") => Ok("linux-arm-v7"),

        ("macos", "x86_64") => Ok("osx-amd64"),

        ("macos", "aarch64") => Ok("osx-arm64"),

        ("windows", "x86_64") => Ok("windows-amd64"),

        _ => Err(format!(
            "BOREAL does not currently support Rclone installation \
                 on OS '{os}' architecture '{arch}'"
        )
        .into()),
    }
}

fn download<F>(url: &str, destination: &Path, progress: F) -> Result<(), RcloneError>
where
    F: FnMut(u64, Option<u64>),
{
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        // Large downloads over institutional proxies can be slow. Allow a
        // generous total window while retaining a ceiling for stalled jobs.
        .timeout(Duration::from_secs(30 * 60))
        .user_agent(concat!("BOREAL/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("Unable to initialize the Rclone downloader: {error}"))?;
    let mut response = client
        .get(url)
        .send()
        .map_err(|error| format!("Unable to download Rclone from {url}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Rclone download returned an error: {error}"))?;
    let mut output = File::create(destination).map_err(|error| {
        format!(
            "Unable to create Rclone download {}: {error}",
            destination.display()
        )
    })?;
    let expected_bytes = response.content_length();
    copy_download(&mut response, &mut output, expected_bytes, progress)?;
    output
        .sync_all()
        .map_err(|error| format!("Unable to finish writing the Rclone download: {error}"))?;
    Ok(())
}

fn copy_download(
    reader: &mut impl Read,
    writer: &mut impl Write,
    expected_bytes: Option<u64>,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<u64, RcloneError> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    let mut next_report = 5 * 1024 * 1024;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("Rclone download was interrupted: {error}"))?;
        if count == 0 {
            break;
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|error| format!("Unable to write the Rclone download: {error}"))?;
        copied = copied.saturating_add(count as u64);
        progress(copied, expected_bytes);
        if copied >= next_report {
            match expected_bytes.filter(|total| *total > 0) {
                Some(total) => println!(
                    "==> Rclone download: {:.0}% ({} of {} MiB)",
                    copied as f64 * 100.0 / total as f64,
                    copied / 1024 / 1024,
                    total / 1024 / 1024,
                ),
                None => println!("==> Rclone download: {} MiB", copied / 1024 / 1024),
            }
            next_report = copied.saturating_add(5 * 1024 * 1024);
        }
    }
    if let Some(expected) = expected_bytes {
        if copied != expected {
            return Err(format!(
                "Rclone download ended early: received {copied} of {expected} bytes"
            )
            .into());
        }
    }
    Ok(copied)
}

fn extract_rclone(archive_path: &Path, destination: &Path) -> Result<(), RcloneError> {
    let archive_file = File::open(archive_path).map_err(|error| {
        format!(
            "Unable to open downloaded Rclone archive {}: {error}",
            archive_path.display()
        )
    })?;

    let mut archive = ZipArchive::new(archive_file).map_err(|error| {
        format!(
            "Unable to read Rclone ZIP archive {}: {error}",
            archive_path.display()
        )
    })?;

    let expected_name = if cfg!(windows) {
        OsStr::new("rclone.exe")
    } else {
        OsStr::new("rclone")
    };

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Unable to read Rclone ZIP entry {index}: {error}"))?;

        let Some(entry_path) = entry.enclosed_name() else {
            continue;
        };

        let Some(file_name) = entry_path.file_name() else {
            continue;
        };

        if file_name != expected_name {
            continue;
        }

        let mut output = File::create(destination).map_err(|error| {
            format!(
                "Unable to create temporary Rclone executable {}: {error}",
                destination.display()
            )
        })?;

        io::copy(&mut entry, &mut output)
            .map_err(|error| format!("Unable to extract Rclone executable: {error}"))?;

        return Ok(());
    }

    Err(format!(
        "Rclone executable was not found inside {}",
        archive_path.display()
    )
    .into())
}

#[cfg(unix)]
fn set_executable_permissions(path: &Path) -> Result<(), RcloneError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();

    permissions.set_mode(0o755);

    fs::set_permissions(path, permissions)?;

    Ok(())
}

#[cfg(windows)]
fn set_executable_permissions(_path: &Path) -> Result<(), RcloneError> {
    Ok(())
}

fn temporary_archive_path() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    env::temp_dir().join(format!(
        "boreal-rclone-{}-{timestamp}.zip",
        std::process::id(),
    ))
}

fn temporary_executable_path(bin_dir: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let name = if cfg!(windows) {
        format!(".rclone-{timestamp}.tmp.exe")
    } else {
        format!(".rclone-{timestamp}.tmp")
    };

    bin_dir.join(name)
}

fn remove_if_exists(path: &Path) -> Result<(), RcloneError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),

        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),

        Err(error) => Err(format!("Unable to remove {}: {error}", path.display()).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::copy_download;
    use std::io::Cursor;

    #[test]
    fn copies_download_bytes_without_an_external_command() {
        let expected = b"rclone archive bytes";
        let mut reader = Cursor::new(expected);
        let mut actual = Vec::new();

        let copied = copy_download(
            &mut reader,
            &mut actual,
            Some(expected.len() as u64),
            |_, _| {},
        )
        .expect("download should copy");

        assert_eq!(copied, expected.len() as u64);
        assert_eq!(actual, expected);
    }
}
