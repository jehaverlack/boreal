use std::{
    ffi::OsStr,
    path::Path,
    process::{Command, Output},
};

use super::RcloneError;

/// Execute the BOREAL-managed Rclone executable.
///
/// This is the central process execution function for the
/// Rclone subsystem.
pub fn run<I, S>(executable: &Path, args: I) -> Result<Output, RcloneError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if !executable.is_file() {
        return Err(format!("Rclone executable does not exist: {}", executable.display()).into());
    }

    let output = Command::new(executable)
        .args(args)
        .output()
        .map_err(|error| {
            format!(
                "Unable to execute Rclone at {}: {error}",
                executable.display()
            )
        })?;

    Ok(output)
}

/// Query the Rclone version.
///
/// Returns the first line produced by:
///
///     rclone version
///
/// Example:
///
///     rclone v1.75.0
///
pub fn version(executable: &Path) -> Result<String, RcloneError> {
    let output = run(executable, ["version"])?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);

        return Err(format!(
            "Rclone version check failed for {}: {}",
            executable.display(),
            stderr.trim()
        )
        .into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let version = stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| {
            format!(
                "Rclone returned no version information: {}",
                executable.display()
            )
        })?;

    Ok(version.to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn existing_remotes_are_used_even_when_retired_shared_auth_files_exist() {
        use std::{fs, os::unix::fs::PermissionsExt};
        let folder = std::env::temp_dir().join(format!(
            "boreal-remote-restore-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&folder).unwrap();
        let config = folder.join("rclone.conf");
        fs::write(
            &config,
            "[my-drive-ro]\ntype = drive\n[my-drive-rw]\ntype = drive\n",
        )
        .unwrap();
        fs::write(folder.join("google-account.json"), "retired and invalid").unwrap();
        fs::write(folder.join("google-connection.json"), "retired and invalid").unwrap();
        let executable = folder.join("fake-rclone");
        fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let output = run(
            &executable,
            [
                "copy",
                "my-drive-ro:source",
                "my-drive-rw:target",
                "--config",
                config.to_str().unwrap(),
            ],
        )
        .unwrap();
        assert!(output.status.success());
        let args = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            args,
            format!(
                "copy\nmy-drive-ro:source\nmy-drive-rw:target\n--config\n{}\n",
                config.display()
            )
        );
        assert_eq!(
            fs::read_to_string(folder.join("google-account.json")).unwrap(),
            "retired and invalid"
        );
        fs::remove_dir_all(folder).unwrap();
    }
}
