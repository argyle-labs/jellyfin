//! Jellyfin deployment lifecycle tool surface.
//!
//! Net-new over the diagnosis surface: these `#[orca_tool]`s own the full
//! deploy lifecycle of a Jellyfin instance — provision, version bump, and
//! config backup/restore — driving `docker compose` and `tar` for the
//! `/config` volume through `plugin_toolkit::process::Command`. LXC runtimes
//! return unsupported: they need an in-guest orca member.
//!
//! Imports flow through `plugin_toolkit::prelude::*` only — the toolkit is the
//! single gateway. Process exec uses the toolkit's orca-owned `process` module.
#![allow(clippy::disallowed_types)]

use std::path::Path;

use plugin_toolkit::prelude::*;
use plugin_toolkit::process::{Command, Output};

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
    /// Proxmox LXC; lifecycle ops require an in-guest orca member.
    Lxc,
    /// Docker / Compose, driven via `docker`.
    #[default]
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

pub(crate) const LXC_UNSUPPORTED: &str = "unsupported: runtime=lxc needs an in-guest orca member; \
     the jellyfin plugin does not run privileged host commands";

/// Resolve the `docker` binary to an absolute path, falling back to the bare
/// name (PATH lookup at spawn) when `which` can't find it. Mirrors the docker
/// plugin so the update/install paths don't silently depend on the daemon's PATH.
fn docker_bin() -> String {
    plugin_toolkit::path::which("docker").unwrap_or_else(|| "docker".to_string())
}

/// The directory to run `docker compose` from — the compose file's own parent —
/// so relative volume/env paths and the derived project name resolve against the
/// deployment dir rather than the daemon's cwd. `None` when the path has no
/// directory component (bare filename), leaving cwd unchanged.
fn compose_dir(compose_file: &str) -> Option<std::path::PathBuf> {
    Path::new(compose_file)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
}

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

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.install — provision a Compose deployment (LXC unsupported)
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
    #[arg(long, value_enum, default_value_t = Runtime::Docker)]
    #[serde(default)]
    pub runtime: Runtime,
    /// Path to the `compose.yml`. Defaults to the repo-relative asset.
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

/// **Provision a Jellyfin deployment.** On `docker`, brings up the Compose
/// stack. On `lxc`, returns unsupported: it needs an in-guest orca member.
#[orca_tool(domain = "jellyfin", verb = "install")]
async fn jellyfin_install(
    args: JellyfinInstallArgs,
    _ctx: &ToolCtx,
) -> Result<JellyfinInstallOutput> {
    install(args).await
}

async fn install(args: JellyfinInstallArgs) -> Result<JellyfinInstallOutput> {
    let output = match args.runtime {
        Runtime::Lxc => bail!(LXC_UNSUPPORTED),
        Runtime::Docker => {
            let compose = args
                .bootstrap_path
                .clone()
                .unwrap_or_else(|| "compose.yml".to_string());
            let mut cmd = Command::new(docker_bin())
                .arg("compose")
                .arg("-f")
                .arg(&compose)
                .arg("up")
                .arg("-d");
            if let Some(dir) = compose_dir(&compose) {
                cmd = cmd.current_dir(dir);
            }
            run(cmd).await?
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
    #[arg(long, value_enum, default_value_t = Runtime::Docker)]
    #[serde(default)]
    pub runtime: Runtime,
    /// Release channel to move to. `latest` / `rc` / `stable`.
    #[arg(long, value_enum, default_value_t = Channel::Latest)]
    #[serde(default)]
    pub channel: Channel,
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
/// `lxc`, returns unsupported: it needs an in-guest orca member. The
/// memory-guard restart (`jellyfin.memory_guard --action recover`) is the
/// separate self-heal path for transcode-memory pressure.
#[orca_tool(domain = "jellyfin", verb = "update")]
async fn jellyfin_update(args: JellyfinUpdateArgs, _ctx: &ToolCtx) -> Result<JellyfinUpdateOutput> {
    update(args).await
}

async fn update(args: JellyfinUpdateArgs) -> Result<JellyfinUpdateOutput> {
    let tag = args.channel.image_tag();
    let log = match args.runtime {
        Runtime::Docker => update_docker(tag, &args.compose_file).await?,
        Runtime::Lxc => bail!(LXC_UNSUPPORTED),
    };
    Ok(JellyfinUpdateOutput {
        updated: true,
        image_tag: tag.to_string(),
        log,
    })
}

/// Docker update: pull the channel image, then `compose up -d` to recreate.
/// Resolves the `docker` binary via PATH lookup (not a bare name that depends on
/// the daemon's PATH) and runs `compose` **from the compose file's own
/// directory** so its relative volume/env paths and derived project name resolve
/// as they do on the deployment host — the bare relative default otherwise
/// resolves against the daemon's cwd.
async fn update_docker(tag: &str, compose_file: &str) -> Result<String> {
    let docker = docker_bin();
    let image = format!("jellyfin/jellyfin:{tag}");
    let pull = run(Command::new(&docker).arg("pull").arg(&image)).await?;
    let mut up = Command::new(&docker)
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("up")
        .arg("-d");
    if let Some(dir) = compose_dir(compose_file) {
        up = up.current_dir(dir);
    }
    let up = run(up).await?;
    Ok(format!(
        "{}\n{}",
        String::from_utf8_lossy(&pull.stdout).trim(),
        String::from_utf8_lossy(&up.stdout).trim()
    )
    .trim()
    .to_string())
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

    #[test]
    fn channel_maps_to_image_tag() {
        assert_eq!(Channel::Latest.image_tag(), "latest");
        assert_eq!(Channel::Rc.image_tag(), "unstable");
        assert_eq!(Channel::Stable.image_tag(), "stable");
    }

    #[tokio::test]
    async fn lxc_install_is_unsupported() {
        let args = JellyfinInstallArgs {
            runtime: Runtime::Lxc,
            bootstrap_path: None,
        };
        let err = install(args).await.unwrap_err().to_string();
        assert!(err.starts_with("unsupported"), "{err}");
    }

    #[tokio::test]
    async fn lxc_update_is_unsupported() {
        let args = JellyfinUpdateArgs {
            runtime: Runtime::Lxc,
            channel: Channel::Latest,
            compose_file: default_compose(),
        };
        let err = update(args).await.unwrap_err().to_string();
        assert!(err.starts_with("unsupported"), "{err}");
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
