use std::{env, ffi::OsStr, thread};

use brr_core::{
    Display, DisplayEnv, Empty, Error, Http, Method, OperationSpec, SandboxMode, Toolset, schema,
};
use brr_store::build;
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::{
    app::Ctx,
    internal,
    services::{
        doctor::{Check, DoctorEnv, backups_check, checks},
        host,
        hosts::{self, capacity},
    },
};

const XVFB: &str = "Xvfb";
const IN_DOCKER: &str = "Not available in the Docker sandbox";
const NO_DESKTOP: &str = "This computer has no desktop";
const NO_XVFB: &str = "Xvfb is not installed";

static INFO: OperationSpec = OperationSpec {
    name: "system.info",
    group: "system",
    toolset: Toolset::System,
    summary: "Daemon version, memory and CPU load of this machine",
    http: Http {
        method: Method::Get,
        path: "/v1/system/info",
    },
    cli: &["system", "info"],
    cli_positional: &[],
    mcp_tool: Some("system_info"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<SystemInfo>,
};

static DOCTOR: OperationSpec = OperationSpec {
    name: "system.doctor",
    group: "system",
    toolset: Toolset::System,
    summary: "Check this machine and the daemon's setup; every problem names its fix",
    http: Http {
        method: Method::Get,
        path: "/v1/system/doctor",
    },
    cli: &["system", "doctor"],
    cli_positional: &[],
    mcp_tool: Some("system_doctor"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<DoctorOutput>,
};

inventory::submit!(OpRegistration {
    spec: &INFO,
    handler: info
});
inventory::submit!(OpRegistration {
    spec: &DOCTOR,
    handler: doctor
});

#[derive(Serialize, JsonSchema)]
struct DoctorOutput {
    checks: Vec<Check>,
}

#[derive(Serialize, JsonSchema)]
struct SystemInfo {
    version: String,
    data_dir: String,
    config_path: String,
    mem_total_mb: u64,
    mem_available_mb: u64,
    logical_cpus: u32,
    load_per_core: f64,
    display_unavailable: Vec<DisplayUnavailable>,
    // Admitted browsers on this machine, against limits.max_instances.
    browsers_running: u32,
    // Admitted browsers on Hosts, and the max of the enabled, reachable Hosts.
    hosts_running: u32,
    hosts_max: u32,
    // True only when this machine and every enabled, reachable Host are full.
    at_capacity: bool,
}

#[derive(Serialize, JsonSchema)]
struct DisplayUnavailable {
    display: Display,
    reason: String,
}

// In the docker sandbox Xvfb runs inside the container, so only visible depends on the host there.
fn display_unavailable(
    sandbox: SandboxMode,
    display_env: &DisplayEnv,
    path_env: &OsStr,
) -> Vec<DisplayUnavailable> {
    let mut unavailable = Vec::new();
    let no_desktop = display_env.display.is_none() && display_env.wayland_display.is_none();
    let visible = match sandbox {
        SandboxMode::Docker => Some(IN_DOCKER),
        SandboxMode::None if no_desktop => Some(NO_DESKTOP),
        SandboxMode::None => None,
    };
    if let Some(reason) = visible {
        unavailable.push(DisplayUnavailable {
            display: Display::Visible,
            reason: reason.to_owned(),
        });
    }

    let has_xvfb = env::split_paths(path_env).any(|dir| dir.join(XVFB).is_file());
    if sandbox == SandboxMode::None && !has_xvfb {
        unavailable.push(DisplayUnavailable {
            display: Display::Virtual,
            reason: NO_XVFB.to_owned(),
        });
    }

    unavailable
}

fn info(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        let memory = host::memory()?;
        let logical_cpus = thread::available_parallelism().map_err(internal)?.get();
        let settings = ctx.app.settings();
        // Hosts take browsers only in the docker sandbox.
        let usable_hosts = match settings.sandbox.mode {
            SandboxMode::Docker => hosts::cached(&ctx.app).await?,
            SandboxMode::None => Vec::new(),
        };
        let counts = capacity(
            &ctx.app.admission.placed().await,
            settings.limits.max_instances,
            &usable_hosts,
        );

        output(&SystemInfo {
            version: ctx.app.version.to_string(),
            data_dir: ctx.app.paths.data_dir.to_string_lossy().into_owned(),
            config_path: ctx.app.paths.config_file().to_string_lossy().into_owned(),
            mem_total_mb: memory.total_mb,
            mem_available_mb: memory.available_mb,
            logical_cpus: u32::try_from(logical_cpus).map_err(internal)?,
            load_per_core: host::load_per_core()?,
            display_unavailable: display_unavailable(
                settings.sandbox.mode,
                &ctx.app.display_env,
                &ctx.app.path_env,
            ),
            browsers_running: counts.local_running,
            hosts_running: counts.hosts_running,
            hosts_max: counts.hosts_max,
            at_capacity: counts.at_capacity,
        })
    }
    .boxed()
}

fn doctor(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        let builds = ctx.app.store.run(|c| build::all(c)).await?;
        let settings = ctx.app.settings();
        let env = DoctorEnv {
            paths: &ctx.app.paths,
            settings: &settings,
            display_env: &ctx.app.display_env,
            path_env: &ctx.app.path_env,
            daemon_running: true,
        };

        let mut found = checks(&env, &builds).await;
        found.push(backups_check(&ctx.app).await?);

        output(&DoctorOutput { checks: found })
    }
    .boxed()
}
