//! Jellyfin tool surface.
//!
//! Endpoint registry: `jellyfin.{list, detail, create, update, delete}` —
//! generated wholesale by `endpoint_resource!`. The macro emits the row
//! struct, db helpers, schema fragment, args/output types, and the five
//! `#[orca_tool]`-annotated functions in one shot. See
//! [[feedback-plugin-toolkit-max-power-min-boilerplate]].
//!
//! Server diagnosis: `jellyfin.server_info`, `jellyfin.libraries`, and the
//! core `jellyfin.transcode_health` are hand-written `#[orca_tool]`s that
//! call out over HTTP through the typed `Client` rather than over the local
//! registry table.
//!
//! Endpoint resolution: every diagnosis tool accepts the endpoint *name* and
//! loads `(base_url, token)` from the toolkit-generated `endpoint_db` at call
//! time. Per [[project-colocated-api-clients]] + model B (any creds-holder may
//! execute), the row syncs to every paired peer so any of them can call
//! `jellyfin.*` against a registered endpoint.
//!
//! Imports flow through `plugin_toolkit::prelude::*` only — the plugin treats
//! the toolkit as the single gateway to the orca system.
#![allow(clippy::disallowed_types)]

use plugin_toolkit::notifications::{Event, EventClass, Severity, emit, registered_backend_names};
use plugin_toolkit::prelude::*;

use crate::diag::SessionTranscodeHealth;
use crate::{Client, Config, ServerInfo, VirtualFolder};

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.{list,detail,create,update,delete} — endpoint registry CRUD.
// One declaration → five tools, three transports each, schema fragment, db
// helpers, row struct, args/output types. Power scales with the macro.
// ═══════════════════════════════════════════════════════════════════════════

#[endpoint_resource(plugin = "jellyfin")]
pub struct JellyfinEndpoint {
    pub name: String,
    pub base_url: String,
    #[secret]
    pub token: String,
    pub enabled: bool,
}

// ── HTTP client helper ──────────────────────────────────────────────────────

fn make_client(name: &str) -> Result<Client> {
    let conn = runtime::open_db()?;
    let row = endpoint_db::get(&conn, name)?
        .with_context(|| format!("jellyfin endpoint '{name}' not registered"))?;
    if !row.enabled {
        bail!("jellyfin endpoint '{name}' is disabled");
    }
    Ok(Client::new(Config::new(row.base_url, row.token)))
}

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.server_info — server name / version / OS
// ═══════════════════════════════════════════════════════════════════════════

#[derive(
    plugin_toolkit::clap::Args,
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
pub struct JellyfinServerInfoArgs {
    /// Registered endpoint name.
    pub endpoint: String,
}

/// Server name, version, and operating system from `/System/Info`.
#[orca_tool(domain = "jellyfin", verb = "server_info")]
async fn jellyfin_server_info(args: JellyfinServerInfoArgs, _ctx: &ToolCtx) -> Result<ServerInfo> {
    Ok(make_client(&args.endpoint)?.server_info().await?)
}

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.libraries — configured libraries / virtual folders
// ═══════════════════════════════════════════════════════════════════════════

#[derive(
    plugin_toolkit::clap::Args,
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
pub struct JellyfinLibrariesArgs {
    /// Registered endpoint name.
    pub endpoint: String,
}

#[derive(
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
pub struct JellyfinLibrariesOutput {
    /// Configured libraries from `/Library/VirtualFolders`.
    pub libraries: Vec<JellyfinLibrary>,
}

/// One configured library, flattened for the tool boundary.
#[derive(
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(rename_all = "camelCase")]
pub struct JellyfinLibrary {
    pub name: Option<String>,
    pub collection_type: Option<String>,
    pub locations: Vec<String>,
    pub item_id: Option<String>,
}

impl From<VirtualFolder> for JellyfinLibrary {
    fn from(v: VirtualFolder) -> Self {
        Self {
            name: v.name,
            collection_type: v.collection_type,
            locations: v.locations.unwrap_or_default(),
            item_id: v.item_id,
        }
    }
}

/// Configured libraries (virtual folders) on a registered Jellyfin server.
#[orca_tool(domain = "jellyfin", verb = "libraries")]
async fn jellyfin_libraries(
    args: JellyfinLibrariesArgs,
    _ctx: &ToolCtx,
) -> Result<JellyfinLibrariesOutput> {
    let libraries = make_client(&args.endpoint)?
        .libraries()
        .await?
        .into_iter()
        .map(JellyfinLibrary::from)
        .collect();
    Ok(JellyfinLibrariesOutput { libraries })
}

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.transcode_health — CORE DIAGNOSIS
//
// `GET /Sessions` → per-session transcode state. A transcoding session whose
// `TranscodingInfo.HardwareAccelerationType` is `none` or absent is running a
// SOFTWARE transcode (CPU fallback) — the condition operators chase. The
// summary surfaces whether *any* session is software-fallback so a caller can
// branch without re-walking the list.
// ═══════════════════════════════════════════════════════════════════════════

#[derive(
    plugin_toolkit::clap::Args,
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
pub struct JellyfinTranscodeHealthArgs {
    /// Registered endpoint name.
    pub endpoint: String,
}

#[derive(
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(rename_all = "camelCase")]
pub struct JellyfinTranscodeHealthOutput {
    /// Total active sessions reported by `/Sessions`.
    pub session_count: usize,
    /// Sessions actively transcoding (have a `TranscodingInfo`).
    pub transcoding_count: usize,
    /// Sessions transcoding on the CPU instead of a hardware encoder/decoder.
    pub software_fallback_count: usize,
    /// True when at least one session is a software fallback — the single
    /// flag a caller branches on to alert "HW accel is not engaging".
    pub any_software_fallback: bool,
    /// Per-session detail.
    pub sessions: Vec<SessionTranscodeHealth>,
}

/// **Core diagnosis.** Classify every active Jellyfin session as
/// direct-play, hardware transcode, or software (CPU) fallback, and flag
/// whether hardware acceleration is failing to engage.
#[orca_tool(domain = "jellyfin", verb = "transcode_health")]
async fn jellyfin_transcode_health(
    args: JellyfinTranscodeHealthArgs,
    _ctx: &ToolCtx,
) -> Result<JellyfinTranscodeHealthOutput> {
    let sessions = make_client(&args.endpoint)?.transcode_health().await?;
    let session_count = sessions.len();
    let transcoding_count = sessions.iter().filter(|s| s.is_transcoding).count();
    let software_fallback_count = sessions.iter().filter(|s| s.software_fallback).count();
    Ok(JellyfinTranscodeHealthOutput {
        session_count,
        transcoding_count,
        software_fallback_count,
        any_software_fallback: software_fallback_count > 0,
        sessions,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// jellyfin.memory_guard — DETECT → REMEDIATE → NOTIFY (self-healing)
//
// Encodes the memory-exhaustion incident as a plugin capability per the
// operator's hard rule: every manual fix becomes detect/remediate/notify in
// the owning plugin, never standalone glue.
//
// DETECT (real I/O): a liveness GET of `/System/Info` distinguishes
// "served fast" from "timed out / 5xx under memory thrash" — the pressure
// signal observable from outside the guest — combined with the count of
// active transcoding sessions (each software transcode is a memory driver).
//
// REMEDIATE (`recover` action only, when pressure is detected): restart the
// Jellyfin application via `POST /System/Restart`, which reaps the transcode
// child processes (the ffmpeg workers driving anon-heap growth) and releases
// the accumulated memory — the service-layer equivalent of the guest restart,
// owned by this plugin. Per the self-healing rule we run the recovery
// sequence and only fail if recovery itself fails; we never refuse because
// the service looks broken. After restart we re-probe liveness to confirm.
//
// NOTIFY: emit a notifications-domain event (info on healthy, warn on
// pressure-only probe, lifecycle on a performed restart, error if recovery
// failed) through the toolkit `notifications` gateway.
// ═══════════════════════════════════════════════════════════════════════════

/// Remediation policy for `jellyfin.memory_guard`.
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
pub enum MemoryGuardAction {
    /// Detect-only: probe liveness + transcode load, never remediate.
    #[default]
    Probe,
    /// Detect, and if memory pressure is found, restart Jellyfin to recover.
    Recover,
}

#[derive(
    plugin_toolkit::clap::Args,
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
pub struct JellyfinMemoryGuardArgs {
    /// Registered endpoint name.
    pub endpoint: String,
    /// What to do when pressure is detected. `probe` (default) only reports;
    /// `recover` restarts Jellyfin to release accumulated memory.
    #[arg(long, value_enum, default_value_t = MemoryGuardAction::Probe)]
    #[serde(default)]
    pub action: MemoryGuardAction,
    /// Active-transcode count at or above which the service is considered
    /// under memory pressure even while still reachable. Each software
    /// transcode is a memory driver; the default catches a runaway pile-up.
    #[arg(long, default_value_t = 4)]
    #[serde(default = "default_transcode_threshold")]
    pub transcode_threshold: usize,
}

fn default_transcode_threshold() -> usize {
    4
}

#[derive(
    plugin_toolkit::serde::Serialize,
    plugin_toolkit::serde::Deserialize,
    plugin_toolkit::schemars::JsonSchema,
)]
#[serde(crate = "plugin_toolkit::serde")]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(rename_all = "camelCase")]
pub struct JellyfinMemoryGuardOutput {
    /// True when the service answered the liveness probe.
    pub reachable: bool,
    /// HTTP status from the liveness probe, when reachable.
    pub liveness_status: Option<u16>,
    /// Active transcoding sessions observed (when reachable).
    pub transcoding_count: usize,
    /// True when the detector judged the service under memory pressure.
    pub pressure_detected: bool,
    /// Why pressure was flagged (unreachable / transcode pile-up), if any.
    pub pressure_reason: Option<String>,
    /// True when a Jellyfin restart was performed this call.
    pub restarted: bool,
    /// Liveness status after a restart, when one was performed and re-probed.
    pub post_restart_status: Option<u16>,
    /// True when recovery completed and the service is reachable again.
    pub recovered: bool,
}

/// **Self-healing memory guard.** Detect Jellyfin memory pressure via real
/// liveness + transcode-load I/O; with `action=recover`, restart the service
/// to release accumulated transcode memory and confirm recovery. Emits a
/// notification of the outcome.
#[orca_tool(domain = "jellyfin", verb = "memory_guard")]
async fn jellyfin_memory_guard(
    args: JellyfinMemoryGuardArgs,
    _ctx: &ToolCtx,
) -> Result<JellyfinMemoryGuardOutput> {
    let client = make_client(&args.endpoint)?;

    // ── DETECT (real I/O) ────────────────────────────────────────────────
    let liveness = client.liveness().await;
    let (reachable, liveness_status) = match &liveness {
        Ok(status) => (true, Some(*status)),
        Err(_) => (false, None),
    };

    let transcoding_count = if reachable {
        match client.transcode_health().await {
            Ok(sessions) => sessions.iter().filter(|s| s.is_transcoding).count(),
            Err(_) => 0,
        }
    } else {
        0
    };

    let pressure_reason = if !reachable {
        Some("liveness probe failed (timeout/5xx under memory thrash)".to_string())
    } else if transcoding_count >= args.transcode_threshold {
        Some(format!(
            "{transcoding_count} active transcodes ≥ threshold {}",
            args.transcode_threshold
        ))
    } else {
        None
    };
    let pressure_detected = pressure_reason.is_some();

    // ── REMEDIATE (recover action, when pressure detected) ───────────────
    let mut restarted = false;
    let mut post_restart_status: Option<u16> = None;
    if pressure_detected && matches!(args.action, MemoryGuardAction::Recover) {
        // Self-healing rule: run the recovery sequence; only fail if the
        // recovery call itself fails — never refuse because the service looks
        // broken. The restart reaps transcode children and frees the heap.
        client
            .restart()
            .await
            .context("jellyfin restart (memory_guard recovery) failed")?;
        restarted = true;
        // Re-probe to confirm recovery. A freshly-restarting server may not
        // answer immediately; an error here means not-yet-recovered, not a
        // tool failure.
        if let Ok(status) = client.liveness().await {
            post_restart_status = Some(status);
        }
    }

    let recovered = if restarted {
        post_restart_status.is_some()
    } else {
        reachable && !pressure_detected
    };

    // ── NOTIFY (notifications gateway) ───────────────────────────────────
    notify_outcome(
        &args.endpoint,
        pressure_detected,
        restarted,
        recovered,
        pressure_reason.as_deref(),
        transcoding_count,
    )
    .await;

    Ok(JellyfinMemoryGuardOutput {
        reachable,
        liveness_status,
        transcoding_count,
        pressure_detected,
        pressure_reason,
        restarted,
        post_restart_status,
        recovered,
    })
}

/// Fan an outcome event through the notifications domain. A soft no-op when no
/// backend is configured on this host.
async fn notify_outcome(
    endpoint: &str,
    pressure_detected: bool,
    restarted: bool,
    recovered: bool,
    reason: Option<&str>,
    transcoding_count: usize,
) {
    if registered_backend_names().is_empty() {
        return;
    }
    let (class, severity, title) = match (pressure_detected, restarted, recovered) {
        (false, _, _) => (
            EventClass::Heartbeat,
            Severity::Info,
            format!("jellyfin '{endpoint}': memory healthy"),
        ),
        (true, true, true) => (
            EventClass::Lifecycle,
            Severity::Warn,
            format!("jellyfin '{endpoint}': memory pressure — restarted, recovered"),
        ),
        (true, true, false) => (
            EventClass::Alert,
            Severity::Error,
            format!("jellyfin '{endpoint}': restarted but not yet reachable"),
        ),
        (true, false, _) => (
            EventClass::Drift,
            Severity::Warn,
            format!("jellyfin '{endpoint}': memory pressure detected (probe-only)"),
        ),
    };
    let body = format!(
        "active transcodes: {transcoding_count}{}",
        reason.map(|r| format!("\nreason: {r}")).unwrap_or_default()
    );
    let event = Event::new(class, severity, title, "jellyfin:memory_guard")
        .with_host(endpoint.to_string())
        .with_body(body);
    let _ = emit(&event).await;
}
