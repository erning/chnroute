use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use tempfile::{Builder as TempDirBuilder, TempDir};

pub(crate) fn publish_staged_directory(staging: TempDir, output: &Path) -> Result<()> {
    let staging_path = staging.keep();

    if !output.exists() {
        return fs::rename(&staging_path, output).with_context(|| {
            format!(
                "failed to publish {} to {}",
                staging_path.display(),
                output.display()
            )
        });
    }

    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let output_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("output directory must have a valid final component"))?;
    let backup_reservation = TempDirBuilder::new()
        .prefix(&format!(".{output_name}.backup-"))
        .tempdir_in(parent)
        .context("failed to reserve rollback directory")?;
    let backup = backup_reservation.path().to_owned();
    backup_reservation
        .close()
        .context("failed to prepare rollback directory")?;

    fs::rename(output, &backup).with_context(|| {
        format!(
            "failed to move existing snapshot {} to {}",
            output.display(),
            backup.display()
        )
    })?;

    if let Err(publish_error) = fs::rename(&staging_path, output) {
        let restore_result = fs::rename(&backup, output);
        let _ = fs::remove_dir_all(&staging_path);
        return match restore_result {
            Ok(()) => Err(publish_error).with_context(|| {
                format!(
                    "failed to publish new snapshot; restored {}",
                    output.display()
                )
            }),
            Err(restore_error) => Err(anyhow!(
                "failed to publish new snapshot ({publish_error}) and failed to restore {} ({restore_error}); backup remains at {}",
                output.display(),
                backup.display()
            )),
        };
    }

    let _ = fs::remove_dir_all(backup);
    Ok(())
}
