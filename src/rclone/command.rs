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

    let args = args
        .into_iter()
        .map(|a| a.as_ref().to_owned())
        .collect::<Vec<_>>();
    let mut command = Command::new(executable);
    command.args(&args);
    let config = args
        .windows(2)
        .find(|a| a[0] == "--config")
        .map(|a| Path::new(&a[1]));
    let google = args.iter().any(|a| {
        let a = a.to_string_lossy();
        a.starts_with("my-drive-ro:")
            || a.starts_with("my-drive-rw:")
            || a.starts_with("my-drive-ro,")
            || a.starts_with("my-drive-rw,")
    });
    let _bridge = if google && args.first().is_some_and(|a| a != "config") {
        config
            .map(|p| crate::google::bridge::Bridge::attach(&mut command, p))
            .transpose()?
            .flatten()
    } else {
        None
    };
    let output = command.output().map_err(|error| {
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
