//! Jellyfin storage drift detection + remediation.
//!
//! Jellyfin's model differs from Plex's, so this is a genuinely different
//! implementation (the two plugins share nothing but the `plugin-toolkit` dep):
//!
//!   - **Authoritative roots are on disk.** Each library folder maps to its
//!     real media path via a `*.mblink` file under the server's `root/default`
//!     tree (e.g. `Movies/movies.mblink` → `/mnt/data/media/movies`). These
//!     files *are* the source of truth, so detection is filesystem-based — no
//!     database and no `sqlite3` (this Jellyfin ships EF-Core `jellyfin.db`
//!     with no CLI).
//!   - **Item paths are re-derived on scan.** Unlike Plex — where stale
//!     per-item paths must be rewritten in the DB — Jellyfin re-walks the roots
//!     and rebuilds item paths when a library is refreshed. So the native
//!     remedy is a rescan, not DB surgery.
//!
//! Two tools:
//!   - `jellyfin.storage_check` — **detect.** Reads the `.mblink` roots + the
//!     live mounts inside the container and flags any root not backed by a live
//!     mount (`mount_missing`) or not resolving on disk (`root_unresolved`).
//!     Read-only.
//!   - `jellyfin.storage_repair` — **remediate.** Dry-run by default. Blocking
//!     issues (missing mount / unresolvable root) can't be fixed by a rescan —
//!     the mount must be restored first — so the tool reports them and refuses.
//!     When the roots are healthy, `apply` triggers `POST /Library/Refresh` so
//!     Jellyfin re-derives item paths and repopulates artwork.
//!
//! The classifier ([`classify`]) is pure and unit-tested; the exec wrappers
//! drive `pct exec` / `docker exec` and hold no logic worth a container to
//! test.
#![allow(clippy::disallowed_types)]

use plugin_toolkit::prelude::*;
use plugin_toolkit::process::Command;

use crate::lifecycle::Runtime;

/// Default server data root holding the `*.mblink` library maps.
const DEFAULT_CONFIG_ROOT: &str = "/var/lib/jellyfin/root/default";

// ═══════════════════════════════════════════════════════════════════════════
// Pure classifier
// ═══════════════════════════════════════════════════════════════════════════

/// Severity of a storage issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(crate = "plugin_toolkit::serde")]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Playback-breaking (unbacked / unresolvable library root).
    Critical,
    /// Degraded but not blocking.
    Warning,
}

/// One detected storage problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(crate = "plugin_toolkit::serde")]
#[serde(rename_all = "camelCase")]
pub struct StorageIssue {
    /// Stable machine kind, e.g. `mount_missing` / `root_unresolved`.
    pub kind: String,
    /// Severity for triage.
    pub severity: Severity,
    /// The library root the issue concerns.
    pub root: String,
    /// Human-readable explanation.
    pub detail: String,
    /// True when a library rescan cannot fix this (the mount must be restored
    /// first). Used by `storage_repair` to refuse a futile rescan.
    pub blocks_rescan: bool,
}

/// Full detect report for one instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(crate = "plugin_toolkit::serde")]
#[serde(rename_all = "camelCase")]
pub struct StorageReport {
    /// True when no issues were found.
    pub healthy: bool,
    /// Authoritative library roots read from the `.mblink` maps.
    pub library_roots: Vec<String>,
    /// Live mount targets observed inside the container.
    pub mount_targets: Vec<String>,
    /// Everything wrong.
    pub issues: Vec<StorageIssue>,
}

/// Split an absolute path into its non-empty segments.
fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// True when `path` is `root` itself or lives beneath it (segment-aligned, so
/// `/mnt/data` does not match `/mnt/database`).
fn is_under(path: &str, root: &str) -> bool {
    let (p, r) = (segments(path), segments(root));
    r.len() <= p.len() && p[..r.len()] == r[..]
}

/// Build the report from gathered facts. Pure — the exec layer feeds it the
/// `.mblink` roots, the live mounts, and the subset of roots that did not
/// resolve on disk.
pub fn classify(
    library_roots: Vec<String>,
    mount_targets: Vec<String>,
    unresolved: &[String],
) -> StorageReport {
    let mut issues = Vec::new();

    for root in &library_roots {
        if !mount_targets.iter().any(|m| is_under(root, m)) {
            issues.push(StorageIssue {
                kind: "mount_missing".to_string(),
                severity: Severity::Critical,
                root: root.clone(),
                detail: format!(
                    "library root '{root}' is not backed by any live mount ({})",
                    if mount_targets.is_empty() {
                        "no mounts observed".to_string()
                    } else {
                        mount_targets.join(", ")
                    }
                ),
                blocks_rescan: true,
            });
        } else if unresolved.iter().any(|u| u == root) {
            issues.push(StorageIssue {
                kind: "root_unresolved".to_string(),
                severity: Severity::Critical,
                root: root.clone(),
                detail: format!("library root '{root}' does not resolve on disk"),
                blocks_rescan: true,
            });
        }
    }

    StorageReport {
        healthy: issues.is_empty(),
        library_roots,
        mount_targets,
        issues,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Exec layer
// ═══════════════════════════════════════════════════════════════════════════

/// Base `pct exec <id> --` / `docker exec <id>` command.
fn guest(runtime: Runtime, id: &str) -> Command {
    match runtime {
        Runtime::Lxc => Command::new("pct").arg("exec").arg(id).arg("--"),
        Runtime::Docker => Command::new("docker").arg("exec").arg(id),
    }
}

/// Run a guest command, returning stdout; a non-zero exit carries stderr.
async fn capture(cmd: Command) -> Result<String> {
    let out = cmd
        .output()
        .await
        .context("failed to spawn guest command")?;
    if !out.status.success {
        bail!(
            "guest command failed (exit {}): {}",
            out.status
                .code
                .map_or_else(|| "signal".to_string(), |c| c.to_string()),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Non-empty, de-duplicated, trimmed lines.
fn lines(out: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for l in out.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !seen.iter().any(|s| s == l) {
            seen.push(l.to_string());
        }
    }
    seen
}

/// Gather the `.mblink` roots + live mounts + unresolved roots, then classify.
async fn check(runtime: Runtime, id: &str, config_root: &str) -> Result<StorageReport> {
    // Each `.mblink` file's contents is a real library path. `-exec sh -c` with
    // a trailing `echo` guarantees a newline between concatenated files.
    let find = guest(runtime, id)
        .arg("find")
        .arg(config_root)
        .arg("-name")
        .arg("*.mblink")
        .arg("-type")
        .arg("f")
        .arg("-exec")
        .arg("sh")
        .arg("-c")
        .arg("cat \"$1\"; echo")
        .arg("_")
        .arg("{}")
        .arg(";");
    let roots = lines(&capture(find).await.context("read .mblink roots")?);

    let mount_cmd = guest(runtime, id).arg("findmnt").arg("-rno").arg("TARGET");
    let mount_targets = lines(&capture(mount_cmd).await.unwrap_or_default());

    // Probe each root's existence individually (paths may contain spaces).
    let mut unresolved = Vec::new();
    for root in &roots {
        let test = guest(runtime, id).arg("test").arg("-d").arg(root);
        if capture(test).await.is_err() {
            unresolved.push(root.clone());
        }
    }

    Ok(classify(roots, mount_targets, &unresolved))
}

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.storage_check — DETECT
// ═══════════════════════════════════════════════════════════════════════════

#[derive(clap::Args, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(crate = "plugin_toolkit::serde")]
pub struct JellyfinStorageCheckArgs {
    /// Where the instance runs: `lxc` or `docker`.
    #[arg(long, value_enum, default_value_t = Runtime::Lxc)]
    #[serde(default)]
    pub runtime: Runtime,
    /// LXC vmid or docker container name/id.
    #[arg(long)]
    pub target: String,
    /// Override the server data root holding the `.mblink` maps.
    #[arg(long, default_value_t = default_config_root())]
    #[serde(default = "default_config_root")]
    pub config_root: String,
}

fn default_config_root() -> String {
    DEFAULT_CONFIG_ROOT.to_string()
}

/// **Detect media-storage problems** on a Jellyfin instance: a library root not
/// backed by a live mount, or one that does not resolve on disk. Read-only.
#[orca_tool(domain = "jellyfin", verb = "storage_check")]
async fn jellyfin_storage_check(
    args: JellyfinStorageCheckArgs,
    _ctx: &ToolCtx,
) -> Result<StorageReport> {
    check(args.runtime, &args.target, &args.config_root).await
}

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.storage_repair — REMEDIATE
// ═══════════════════════════════════════════════════════════════════════════

#[derive(clap::Args, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(crate = "plugin_toolkit::serde")]
pub struct JellyfinStorageRepairArgs {
    /// Where the instance runs: `lxc` or `docker`.
    #[arg(long, value_enum, default_value_t = Runtime::Lxc)]
    #[serde(default)]
    pub runtime: Runtime,
    /// LXC vmid or docker container name/id.
    #[arg(long)]
    pub target: String,
    /// Registered Jellyfin endpoint name, used to trigger the rescan.
    #[arg(long)]
    pub endpoint: String,
    /// Override the server data root holding the `.mblink` maps.
    #[arg(long, default_value_t = default_config_root())]
    #[serde(default = "default_config_root")]
    pub config_root: String,
    /// Trigger the rescan. Without this the tool only reports what it would do.
    #[arg(long)]
    #[serde(default)]
    pub apply: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(crate = "plugin_toolkit::serde")]
#[serde(rename_all = "camelCase")]
pub struct JellyfinStorageRepairOutput {
    /// True when a rescan was triggered.
    pub rescan_triggered: bool,
    /// What happened (or would happen on `apply`).
    pub action: String,
    /// Issues that block a rescan (mount must be restored first).
    pub blocking_issues: Vec<StorageIssue>,
}

/// **Remediate storage state.** Re-runs the check; if any issue blocks a rescan
/// (missing mount / unresolvable root) the tool reports it and does nothing —
/// the mount must be restored first. Otherwise, with `apply`, triggers
/// `POST /Library/Refresh` so Jellyfin re-derives item paths and repopulates
/// artwork. Dry-run by default.
#[orca_tool(domain = "jellyfin", verb = "storage_repair")]
async fn jellyfin_storage_repair(
    args: JellyfinStorageRepairArgs,
    _ctx: &ToolCtx,
) -> Result<JellyfinStorageRepairOutput> {
    let report = check(args.runtime, &args.target, &args.config_root).await?;
    let blocking: Vec<StorageIssue> = report
        .issues
        .into_iter()
        .filter(|i| i.blocks_rescan)
        .collect();

    if !blocking.is_empty() {
        return Ok(JellyfinStorageRepairOutput {
            rescan_triggered: false,
            action: "refused: restore the missing/unresolved mount(s) before rescanning"
                .to_string(),
            blocking_issues: blocking,
        });
    }

    if !args.apply {
        return Ok(JellyfinStorageRepairOutput {
            rescan_triggered: false,
            action: "would trigger POST /Library/Refresh (rescan) — pass --apply".to_string(),
            blocking_issues: Vec::new(),
        });
    }

    crate::tools::make_client(&args.endpoint)?
        .refresh_libraries()
        .await?;
    Ok(JellyfinStorageRepairOutput {
        rescan_triggered: true,
        action: "triggered POST /Library/Refresh".to_string(),
        blocking_issues: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn is_under_is_segment_aligned() {
        assert!(is_under("/mnt/data/media/movies", "/mnt/data"));
        assert!(!is_under("/mnt/database/x", "/mnt/data"));
    }

    #[test]
    fn healthy_when_roots_backed_and_resolve() {
        let report = classify(
            s(&["/mnt/data/media/movies", "/mnt/data/media/tv"]),
            s(&["/mnt/data"]),
            &[],
        );
        assert!(report.healthy);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn flags_root_not_backed_by_mount() {
        let report = classify(s(&["/mnt/data/media/movies"]), s(&["/mnt/backups"]), &[]);
        assert!(!report.healthy);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, "mount_missing");
        assert!(report.issues[0].blocks_rescan);
    }

    #[test]
    fn flags_backed_but_unresolved_root() {
        // Mounted, but the specific library subdir does not exist (e.g. the
        // mount is present but the media subtree moved).
        let report = classify(
            s(&["/mnt/data/media/movies"]),
            s(&["/mnt/data"]),
            &s(&["/mnt/data/media/movies"]),
        );
        assert!(!report.healthy);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, "root_unresolved");
    }

    #[test]
    fn unresolved_under_missing_mount_reports_mount_not_unresolved() {
        // When the mount itself is absent, prefer the mount_missing diagnosis
        // over a redundant unresolved one.
        let report = classify(
            s(&["/mnt/data/media/movies"]),
            s(&["/mnt/backups"]),
            &s(&["/mnt/data/media/movies"]),
        );
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, "mount_missing");
    }

    #[test]
    fn lines_trims_and_dedupes() {
        assert_eq!(lines("  /a \n\n/a\n/b\n"), s(&["/a", "/b"]));
    }
}
