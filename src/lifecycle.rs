//! Jellyfin deployment lifecycle tool surface.
//!
//! Net-new over the diagnosis surface: these `#[orca_tool]`s own the full
//! deploy lifecycle of a Jellyfin instance — provision, version bump, and
//! config backup/restore — driving the host's container runtime
//! (`pct` for Proxmox LXC, `docker` for Compose) and `tar` for the `/config`
//! volume through `tokio::process::Command`. There is no parallel shell glue:
//! the bootstrap scripts in `scripts/` + `lxc/` are the curl-bootstrap payload
//! these tools orchestrate, and every capability is reachable as an orca tool.
//!
//! Imports flow through `plugin_toolkit::prelude::*` only — the toolkit is the
//! single gateway. Process exec uses the toolkit's re-exported `tokio`.
#![allow(clippy::disallowed_types)]

use std::path::Path;
use std::process::Output;

use plugin_toolkit::prelude::*;
use plugin_toolkit::tokio::process::Command;

/// Where a Jellyfin instance is deployed — selects which runtime the lifecycle
/// tools drive.
#[derive(
    Debug,
    Clone,
    Copy,
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
    plugin_toolkit::clap::ValueEnum,
    Default,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    /// Proxmox LXC, driven via `pct`.
    #[default]
    Lxc,
    /// Docker / Compose, driven via `docker`.
    Docker,
}

/// Release channel for `jellyfin.update`. Maps to an image/package version.
#[derive(
    Clone,
    Copy,
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
    plugin_toolkit::clap::ValueEnum,
    Default,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Newest published Jellyfin release.
    #[default]
    Latest,
    /// Release-candidate / unstable channel.
    Rc,
    /// Pinned stable line.
    Stable,
}

impl Channel {
    /// The container image tag this channel resolves to. Jellyfin publishes
    /// `latest`, `unstable`, and version-pinned tags on `jellyfin/jellyfin`.
    fn image_tag(self) -> &'static str {
        match self {
            Channel::Latest => "latest",
            Channel::Rc => "unstable",
            Channel::Stable => "stable",
        }
    }
}

/// Run a command, capturing output, and map a non-zero exit to an error that
/// carries stderr — the lifecycle tools surface the runtime's own message
/// rather than a bare exit code.
async fn run(cmd: &mut Command) -> Result<Output> {
    let output = cmd
        .output()
        .await
        .with_context(|| "failed to spawn command".to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "command failed ({}): {}",
            output.status,
            stderr.trim()
        );
    }
    Ok(output)
}

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.install — provision an LXC or Compose deployment
// ═══════════════════════════════════════════════════════════════════════════

#[derive(
    plugin_toolkit::clap::Args,
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
pub struct JellyfinInstallArgs {
    /// Where to deploy: `lxc` (Proxmox) or `docker` (Compose).
    #[arg(long, value_enum, default_value_t = Runtime::Lxc)]
    #[serde(default)]
    pub runtime: Runtime,
    /// LXC vmid (LXC runtime only). Required when `runtime=lxc`.
    #[arg(long)]
    #[serde(default)]
    pub vmid: Option<u32>,
    /// Host path for the persistent `/config` volume.
    #[arg(long, default_value = "/opt/jellyfin/config")]
    #[serde(default = "default_config_path")]
    pub config_path: String,
    /// Host path to the media library, mounted read-only.
    #[arg(long)]
    #[serde(default)]
    pub media_path: Option<String>,
    /// Skip GPU passthrough (software-transcode-only minimal deploy).
    #[arg(long)]
    #[serde(default)]
    pub no_gpu: bool,
    /// Path to the bootstrap `provision.sh` (LXC) or `compose.yml` (Docker).
    /// Defaults to the repo-relative asset; override for a non-standard layout.
    #[arg(long)]
    #[serde(default)]
    pub bootstrap_path: Option<String>,
}

fn default_config_path() -> String {
    "/opt/jellyfin/config".to_string()
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
pub struct JellyfinInstallOutput {
    /// True when the provisioning command completed successfully.
    pub provisioned: bool,
    /// The runtime the deployment targeted.
    pub runtime: Runtime,
    /// Combined stdout from the provisioning step.
    pub log: String,
}

/// **Provision a Jellyfin deployment.** On `lxc`, runs the Proxmox
/// `provision.sh` bootstrap (create CT, GPU passthrough, install, start). On
/// `docker`, brings up the Compose stack. GPU passthrough is wired by default;
/// pass `no_gpu` for a software-only minimal install.
#[orca_tool(domain = "jellyfin", verb = "install")]
async fn jellyfin_install(
    args: JellyfinInstallArgs,
    _ctx: &ToolCtx,
) -> Result<JellyfinInstallOutput> {
    let output = match args.runtime {
        Runtime::Lxc => {
            let vmid = args
                .vmid
                .context("`vmid` is required when runtime=lxc")?;
            let script = args
                .bootstrap_path
                .clone()
                .unwrap_or_else(|| "lxc/provision.sh".to_string());
            let mut cmd = Command::new("bash");
            cmd.arg(&script).arg(vmid.to_string());
            cmd.arg("--config").arg(&args.config_path);
            if let Some(media) = &args.media_path {
                cmd.arg("--media").arg(media);
            }
            if args.no_gpu {
                cmd.arg("--no-gpu");
            }
            run(&mut cmd).await?
        }
        Runtime::Docker => {
            let compose = args
                .bootstrap_path
                .clone()
                .unwrap_or_else(|| "compose.yml".to_string());
            let mut cmd = Command::new("docker");
            cmd.arg("compose").arg("-f").arg(&compose).arg("up").arg("-d");
            run(&mut cmd).await?
        }
    };
    Ok(JellyfinInstallOutput {
        provisioned: true,
        runtime: args.runtime,
        log: String::from_utf8_lossy(&output.stdout).into_owned(),
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.update — channel-aware image/version bump
// ═══════════════════════════════════════════════════════════════════════════

#[derive(
    plugin_toolkit::clap::Args,
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
pub struct JellyfinUpdateArgs {
    /// Where the instance runs: `lxc` or `docker`.
    #[arg(long, value_enum, default_value_t = Runtime::Lxc)]
    #[serde(default)]
    pub runtime: Runtime,
    /// Release channel to move to. `latest` / `rc` / `stable`.
    #[arg(long, value_enum, default_value_t = Channel::Latest)]
    #[serde(default)]
    pub channel: Channel,
    /// LXC vmid (LXC runtime only).
    #[arg(long)]
    #[serde(default)]
    pub vmid: Option<u32>,
    /// Compose file (Docker runtime only).
    #[arg(long, default_value = "compose.yml")]
    #[serde(default = "default_compose")]
    pub compose_file: String,
}

fn default_compose() -> String {
    "compose.yml".to_string()
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
pub struct JellyfinUpdateOutput {
    /// True when the update command completed.
    pub updated: bool,
    /// Image tag the channel resolved to.
    pub image_tag: String,
    /// Combined stdout from the update step.
    pub log: String,
}

/// **Update a Jellyfin deployment** to the head of a release channel. On
/// `docker`, re-pulls the channel image tag and recreates the container. On
/// `lxc`, runs the in-CT package upgrade. The memory-guard restart capability
/// (`jellyfin.memory_guard --action recover`) is the separate self-heal path
/// for transcode-memory pressure and is intentionally kept distinct from this
/// version bump.
#[orca_tool(domain = "jellyfin", verb = "update")]
async fn jellyfin_update(args: JellyfinUpdateArgs, _ctx: &ToolCtx) -> Result<JellyfinUpdateOutput> {
    let tag = args.channel.image_tag();
    let output = match args.runtime {
        Runtime::Docker => {
            let image = format!("jellyfin/jellyfin:{tag}");
            run(Command::new("docker").arg("pull").arg(&image)).await?;
            run(Command::new("docker")
                .arg("compose")
                .arg("-f")
                .arg(&args.compose_file)
                .arg("up")
                .arg("-d"))
            .await?
        }
        Runtime::Lxc => {
            let vmid = args.vmid.context("`vmid` is required when runtime=lxc")?;
            run(Command::new("pct").arg("exec").arg(vmid.to_string()).arg("--").arg("bash").arg(
                "-c",
            ).arg(
                "apt-get update && apt-get install -y --only-upgrade jellyfin && systemctl restart jellyfin",
            ))
            .await?
        }
    };
    Ok(JellyfinUpdateOutput {
        updated: true,
        image_tag: tag.to_string(),
        log: String::from_utf8_lossy(&output.stdout).into_owned(),
    })
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

/// UTC timestamp `YYYYMMDD-HHMMSS` for archive names. Uses chrono (already a
/// plugin dep via progenitor's date-time formats).
fn now_stamp() -> String {
    chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_maps_to_image_tag() {
        assert_eq!(Channel::Latest.image_tag(), "latest");
        assert_eq!(Channel::Rc.image_tag(), "unstable");
        assert_eq!(Channel::Stable.image_tag(), "stable");
    }

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
