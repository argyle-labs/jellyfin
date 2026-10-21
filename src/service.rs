#![allow(clippy::disallowed_types)]

use plugin_toolkit::prelude::*;
use plugin_toolkit::service::{
    BoxFuture, EnvVar, Mount, Routes, Runtime, ServiceBackend, ServiceCapability, ServiceError,
    WorkloadSpec,
};

pub const PROVIDER: &str = "jellyfin";
pub const IMAGE: &str = "gitea.scottkey.me/argyle-labs/jellyfin:latest";
pub const PORT: u16 = 8096;
pub const MEDIA_ROOT: &str = "/mnt/media";

#[derive(Debug, Clone, Default)]
pub struct JellyfinBackend;

pub fn workload(
    runtime: Runtime,
    instance: &str,
    routes: &Routes,
) -> Result<WorkloadSpec, ServiceError> {
    match runtime {
        Runtime::Docker | Runtime::Podman => {}
        Runtime::Lxc | Runtime::Vm => {
            return Err(ServiceError::Unsupported(
                format!("workload_spec({})", runtime.as_str()),
                PROVIDER.to_string(),
            ))
        }
    }
    let root = format!("/opt/{instance}");
    let mut localtime = Mount::bind("/etc/localtime", "/etc/localtime");
    localtime.read_only = true;
    let mut media = Mount::bind(MEDIA_ROOT, MEDIA_ROOT);
    media.read_only = true;
    let url = routes.primary_url();
    let env = if url.is_empty() {
        Vec::new()
    } else {
        vec![EnvVar {
            key: "JELLYFIN_PublishedServerUrl".to_string(),
            value: url,
        }]
    };
    Ok(WorkloadSpec {
        name: instance.to_string(),
        image: Some(IMAGE.to_string()),
        env,
        mounts: vec![
            localtime,
            Mount::bind(format!("{root}/config"), "/config"),
            Mount::bind(format!("{root}/cache"), "/cache"),
            media,
        ],
        ports: vec![format!("{}:{PORT}", routes.publish_port(PORT))],
    })
}

impl ServiceBackend for JellyfinBackend {
    fn provider(&self) -> &str {
        PROVIDER
    }

    fn runtimes(&self) -> Vec<Runtime> {
        vec![Runtime::Docker, Runtime::Podman]
    }

    fn capabilities(&self) -> Vec<ServiceCapability> {
        vec![ServiceCapability::Deploy]
    }

    fn default_port(&self) -> u16 {
        PORT
    }

    fn data_paths(&self) -> Vec<String> {
        vec!["/config".to_string()]
    }

    fn workload_spec<'a>(
        &'a self,
        runtime: Runtime,
        instance: &'a str,
        routes: &'a Routes,
    ) -> BoxFuture<'a, Result<WorkloadSpec, ServiceError>> {
        Box::pin(async move { workload(runtime, instance, routes) })
    }
}

#[derive(clap::Args, Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(crate = "plugin_toolkit::serde")]
#[serde(rename_all = "camelCase")]
pub struct JellyfinDeployArgs {
    #[arg(long, default_value = PROVIDER)]
    #[serde(default = "default_instance")]
    pub instance: String,
    #[arg(long, default_value = "docker")]
    #[serde(default = "default_runtime")]
    pub runtime: String,
    #[arg(long, default_value = "")]
    #[serde(default)]
    pub host: String,
    #[arg(long = "route")]
    #[serde(default)]
    pub routes: Vec<String>,
    #[arg(long)]
    #[serde(default)]
    pub target_id: Option<String>,
}

fn default_instance() -> String {
    PROVIDER.to_string()
}

fn default_runtime() -> String {
    "docker".to_string()
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(crate = "plugin_toolkit::serde")]
#[serde(rename_all = "camelCase")]
pub struct ServiceCreateRequest {
    pub action: String,
    pub service: String,
    pub instance: String,
    pub runtime: String,
    pub host: String,
    pub routes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
#[schemars(crate = "plugin_toolkit::schemars")]
#[serde(crate = "plugin_toolkit::serde")]
#[serde(rename_all = "camelCase")]
pub struct JellyfinDeployOutput {
    pub executed: bool,
    pub tool: String,
    pub args: ServiceCreateRequest,
}

pub fn deploy_request(args: JellyfinDeployArgs) -> Result<ServiceCreateRequest> {
    let runtime = plugin_toolkit::service::parse_runtime(&args.runtime)?;
    workload(runtime, &args.instance, &Routes::new())?;
    if args.instance.trim().is_empty() {
        bail!("`instance` must not be blank");
    }
    let target_id = match args.target_id {
        Some(id) if id.trim().is_empty() => bail!("`targetId` must not be blank"),
        other => other,
    };
    Ok(ServiceCreateRequest {
        action: "deploy".to_string(),
        service: PROVIDER.to_string(),
        instance: args.instance,
        runtime: plugin_toolkit::service::runtime_str(runtime),
        host: args.host,
        routes: args.routes,
        target_id,
    })
}

fn deploy_output(args: JellyfinDeployArgs) -> Result<JellyfinDeployOutput> {
    Ok(JellyfinDeployOutput {
        executed: false,
        tool: "service.create".to_string(),
        args: deploy_request(args)?,
    })
}

/// **Deploy Jellyfin** through orca's runtime-agnostic `service.create action=deploy`.
/// Returns the exact call to run; plugins cannot yet invoke core tools.
#[orca_tool(domain = "jellyfin", verb = "install")]
async fn jellyfin_install(
    args: JellyfinDeployArgs,
    _ctx: &ToolCtx,
) -> Result<JellyfinDeployOutput> {
    deploy_output(args)
}

/// **Update Jellyfin** by redeploying the current image via `service.create action=deploy`.
/// Returns the exact call to run; plugins cannot yet invoke core tools.
#[orca_tool(domain = "jellyfin", verb = "update")]
async fn jellyfin_update(args: JellyfinDeployArgs, _ctx: &ToolCtx) -> Result<JellyfinDeployOutput> {
    deploy_output(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_toolkit::deploy_target::MountKind;
    use plugin_toolkit::service::Route;

    fn args() -> JellyfinDeployArgs {
        JellyfinDeployArgs {
            instance: "jellyfin".into(),
            runtime: "docker".into(),
            host: "media-host".into(),
            routes: vec!["lan_v4:http://10.0.0.5:18096".into()],
            target_id: Some("0192f0c4-0000-7000-8000-000000000001".into()),
        }
    }

    #[test]
    fn docker_spec_is_the_jellyfin_workload() {
        let routes: Routes = vec![Route::new("lan_v4", "http", "10.0.0.5", Some(18096))].into();
        let spec = workload(Runtime::Docker, "jellyfin", &routes).unwrap();
        assert_eq!(spec.name, "jellyfin");
        assert_eq!(
            spec.image.as_deref(),
            Some("gitea.scottkey.me/argyle-labs/jellyfin:latest")
        );
        assert_eq!(spec.ports, vec!["18096:8096".to_string()]);
        let m: Vec<_> = spec
            .mounts
            .iter()
            .map(|m| (m.source.as_str(), m.target.as_str(), m.read_only, m.kind))
            .collect();
        assert_eq!(
            m,
            vec![
                ("/etc/localtime", "/etc/localtime", true, MountKind::Bind),
                ("/opt/jellyfin/config", "/config", false, MountKind::Bind),
                ("/opt/jellyfin/cache", "/cache", false, MountKind::Bind),
                ("/mnt/media", "/mnt/media", true, MountKind::Bind),
            ]
        );
        assert_eq!(spec.env.len(), 1);
        assert_eq!(spec.env[0].key, "JELLYFIN_PublishedServerUrl");
        assert_eq!(spec.env[0].value, "http://10.0.0.5:18096");
    }

    #[test]
    fn podman_matches_docker_and_defaults_port() {
        let d = workload(Runtime::Docker, "jf2", &Routes::new()).unwrap();
        let p = workload(Runtime::Podman, "jf2", &Routes::new()).unwrap();
        assert_eq!(d.ports, vec!["8096:8096".to_string()]);
        assert_eq!(p.ports, d.ports);
        assert_eq!(p.mounts[1].source, "/opt/jf2/config");
        assert!(p.env.is_empty());
    }

    #[test]
    fn lxc_and_vm_are_unsupported() {
        for rt in [Runtime::Lxc, Runtime::Vm] {
            let err = workload(rt, "jellyfin", &Routes::new()).unwrap_err();
            assert!(matches!(err, ServiceError::Unsupported(..)), "{err}");
        }
    }

    #[test]
    fn backend_declares_container_runtimes_only() {
        let b = JellyfinBackend;
        assert_eq!(b.provider(), "jellyfin");
        assert_eq!(b.default_port(), 8096);
        assert_eq!(b.runtimes(), vec![Runtime::Docker, Runtime::Podman]);
        assert_eq!(b.capabilities(), vec![ServiceCapability::Deploy]);
    }

    #[tokio::test]
    async fn backend_workload_spec_delegates() {
        let spec = JellyfinBackend
            .workload_spec(Runtime::Docker, "jellyfin", &Routes::new())
            .await
            .unwrap();
        assert_eq!(spec.image.as_deref(), Some(IMAGE));
    }

    #[test]
    fn install_builds_the_service_create_call() {
        let out = deploy_output(args()).unwrap();
        assert!(!out.executed);
        assert_eq!(out.tool, "service.create");
        let v = plugin_toolkit::serde_json::to_value(&out.args).unwrap();
        assert_eq!(
            v,
            plugin_toolkit::serde_json::json!({
                "action": "deploy",
                "service": "jellyfin",
                "instance": "jellyfin",
                "runtime": "docker",
                "host": "media-host",
                "routes": ["lan_v4:http://10.0.0.5:18096"],
                "targetId": "0192f0c4-0000-7000-8000-000000000001"
            })
        );
    }

    #[test]
    fn target_id_is_optional_but_never_blank() {
        let mut a = args();
        a.target_id = None;
        let v = plugin_toolkit::serde_json::to_value(deploy_request(a).unwrap()).unwrap();
        assert!(v.get("targetId").is_none());
        let mut a = args();
        a.target_id = Some("  ".into());
        assert!(deploy_request(a).is_err());
    }

    #[test]
    fn install_rejects_unsupported_runtimes() {
        for rt in ["lxc", "vm", "bogus"] {
            let mut a = args();
            a.runtime = rt.into();
            assert!(deploy_request(a).is_err(), "{rt}");
        }
    }

    #[test]
    fn install_and_update_spawn_no_processes() {
        let src = include_str!("service.rs");
        for needle in [
            concat!("Command", "::"),
            concat!("process", "::"),
            concat!("su", "do"),
            concat!("\"p", "ct\""),
        ] {
            assert!(!src.contains(needle), "{needle}");
        }
    }
}
