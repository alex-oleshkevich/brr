use std::thread;

use brr_core::{Empty, Error, Http, Method, OperationSpec, Toolset, schema};
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
        doctor::{Check, DoctorEnv, checks},
        host,
    },
};

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
}

fn info(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        let memory = host::memory()?;
        let logical_cpus = thread::available_parallelism().map_err(internal)?.get();

        output(&SystemInfo {
            version: ctx.app.version.to_string(),
            data_dir: ctx.app.paths.data_dir.to_string_lossy().into_owned(),
            config_path: ctx.app.paths.config_file().to_string_lossy().into_owned(),
            mem_total_mb: memory.total_mb,
            mem_available_mb: memory.available_mb,
            logical_cpus: u32::try_from(logical_cpus).map_err(internal)?,
            load_per_core: host::load_per_core()?,
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

        output(&DoctorOutput {
            checks: checks(&env, &builds).await,
        })
    }
    .boxed()
}
