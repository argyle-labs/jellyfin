//! Jellyfin deployment lifecycle tool surface.
//!
//! Config backup/restore of the `/config` volume via `tar`. Deploy and update
//! go through orca core's `service.create action=deploy` (see `service`).
//!
//! Imports flow through `plugin_toolkit::prelude::*` only — the toolkit is the
//! single gateway. Process exec uses the toolkit's orca-owned `process` module.
#![allow(clippy::disallowed_types)]

use std::path::Path;

use plugin_toolkit::prelude::*;
use plugin_toolkit::process::{Command, Output};

/// Run a command, capturing output, and map a non-zero exit to an error that
/// carries stderr — the lifecycle tools surface the runtime's own message
/// rather than a bare exit code.
async fn run(cmd: Command) -> Result<Output> {
    let output = cmd
        .output()
        .await
        .with_context(|| "failed to spawn command".to_string())?;
    if !output.status.success {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "command failed (exit {}): {}",
            output
                .status
                .code
                .map_or_else(|| "signal".to_string(), |c| c.to_string()),
            stderr.trim()
        );
    }
    Ok(output)
}

fn default_config_path() -> String {
    "/opt/jellyfin/config".to_string()
}

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.backup — tar the /config volume to a destination
// ═══════════════════════════════════════════════════════════════════════════

#[derive(
    plugin_toolkit::clap::Args,
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
pub struct JellyfinBackupArgs {
    /// Host path of the Jellyfin `/config` volume to archive.
    #[arg(long, default_value = "/opt/jellyfin/config")]
    #[serde(default = "default_config_path")]
    pub config_path: String,
    /// Directory to write the `.tar.gz` into. Created if missing.
    #[arg(long)]
    pub destination: String,
}

#[derive(
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(rename_all = "camelCase")]
#[derive(Debug)]
pub struct JellyfinBackupOutput {
    /// Absolute path of the archive written.
    pub archive: String,
}

/// **Back up the Jellyfin `/config` volume** to a `.tar.gz` in the destination
/// directory. The Jellyfin-regenerated `cache/` and `transcodes/` trees are
/// excluded — only durable config/metadata/db is archived.
#[orca_tool(domain = "jellyfin", verb = "backup")]
async fn jellyfin_backup(args: JellyfinBackupArgs, _ctx: &ToolCtx) -> Result<JellyfinBackupOutput> {
    backup_config(&args).await
}

/// Archive logic, independent of the tool context so it is directly testable.
async fn backup_config(args: &JellyfinBackupArgs) -> Result<JellyfinBackupOutput> {
    let config = Path::new(&args.config_path);
    if !config.is_dir() {
        bail!("config path '{}' is not a directory", args.config_path);
    }
    run(Command::new("mkdir").arg("-p").arg(&args.destination)).await?;

    let stamp = now_stamp();
    let archive = format!(
        "{}/jellyfin-config-{}.tar.gz",
        args.destination.trim_end_matches('/'),
        stamp
    );

    run(Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("--exclude=./cache")
        .arg("--exclude=./transcodes")
        .arg("--exclude=./log")
        .arg("-C")
        .arg(&args.config_path)
        .arg("."))
    .await?;

    Ok(JellyfinBackupOutput { archive })
}

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.restore — restore the /config volume from a tarball
// ═══════════════════════════════════════════════════════════════════════════

#[derive(
    plugin_toolkit::clap::Args,
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
pub struct JellyfinRestoreArgs {
    /// The backup tarball to restore from.
    #[arg(long = "from")]
    pub from: String,
    /// Host path of the `/config` volume to restore into. Created if missing.
    #[arg(long, default_value = "/opt/jellyfin/config")]
    #[serde(default = "default_config_path")]
    pub config_path: String,
}

#[derive(
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(rename_all = "camelCase")]
#[derive(Debug)]
pub struct JellyfinRestoreOutput {
    /// True when extraction completed.
    pub restored: bool,
    /// Where the config was restored to.
    pub config_path: String,
}

/// **Restore the Jellyfin `/config` volume** from a `.tar.gz` produced by
/// `jellyfin.backup`. The service should be stopped before restoring; this
/// tool only extracts the archive over the config directory.
#[orca_tool(domain = "jellyfin", verb = "restore")]
async fn jellyfin_restore(
    args: JellyfinRestoreArgs,
    _ctx: &ToolCtx,
) -> Result<JellyfinRestoreOutput> {
    restore_config(args).await
}

/// Extraction logic, independent of the tool context so it is directly testable.
async fn restore_config(args: JellyfinRestoreArgs) -> Result<JellyfinRestoreOutput> {
    if !Path::new(&args.from).is_file() {
        bail!("backup tarball '{}' not found", args.from);
    }
    run(Command::new("mkdir").arg("-p").arg(&args.config_path)).await?;
    run(Command::new("tar")
        .arg("-xzf")
        .arg(&args.from)
        .arg("-C")
        .arg(&args.config_path))
    .await?;
    Ok(JellyfinRestoreOutput {
        restored: true,
        config_path: args.config_path,
    })
}

/// UTC timestamp `YYYYMMDD-HHMMSS` for archive names. Wall-clock time is reached
/// through the toolkit's orca-owned `time` module so the plugin carries no
/// direct datetime dep.
fn now_stamp() -> String {
    plugin_toolkit::time::now().compact()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn backup_rejects_missing_config_dir() {
        let args = JellyfinBackupArgs {
            config_path: "/nonexistent/jellyfin/config/path".to_string(),
            destination: "/tmp/jellyfin-test-dest".to_string(),
        };
        let err = backup_config(&args).await.unwrap_err();
        assert!(err.to_string().contains("not a directory"), "{err}");
    }

    #[tokio::test]
    async fn restore_rejects_missing_tarball() {
        let args = JellyfinRestoreArgs {
            from: "/nonexistent/backup.tar.gz".to_string(),
            config_path: "/tmp/jellyfin-test-restore".to_string(),
        };
        let err = restore_config(args).await.unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[tokio::test]
    async fn backup_then_restore_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("config");
        std::fs::create_dir_all(config.join("data")).unwrap();
        std::fs::write(config.join("system.xml"), b"<config/>").unwrap();
        std::fs::write(config.join("data").join("library.db"), b"sqlite").unwrap();
        // cache must be excluded
        std::fs::create_dir_all(config.join("cache")).unwrap();
        std::fs::write(config.join("cache").join("junk"), b"x").unwrap();

        let dest = tmp.path().join("backups");
        let out = backup_config(&JellyfinBackupArgs {
            config_path: config.to_string_lossy().into_owned(),
            destination: dest.to_string_lossy().into_owned(),
        })
        .await
        .unwrap();
        assert!(Path::new(&out.archive).is_file());

        let restore_target = tmp.path().join("restored");
        restore_config(JellyfinRestoreArgs {
            from: out.archive.clone(),
            config_path: restore_target.to_string_lossy().into_owned(),
        })
        .await
        .unwrap();

        assert!(restore_target.join("system.xml").is_file());
        assert!(restore_target.join("data").join("library.db").is_file());
        // cache was excluded from the archive
        assert!(!restore_target.join("cache").exists());
    }
}
