use std::time::Duration;

use brr_core::{Error, Http, Instance, Method, OperationSpec, Tab, Toolset, schema};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, instances::instance_view, invalid, output, parse, tabs::tab_view};
use crate::{app::Ctx, services::instances::AttachSource};

const EXTENSION_WAIT: Duration = Duration::from_secs(30);
const ATTACH_CHOICE: &str = "pass exactly one of cdp_url, install_name or via";

static ATTACH: OperationSpec = OperationSpec {
    name: "instance.attach",
    group: "instance",
    toolset: Toolset::Instances,
    summary: "Attach to a browser you already run: a CDP endpoint or an install name; its open pages become Tabs",
    http: Http {
        method: Method::Post,
        path: "/v1/instances/attach",
    },
    cli: &["instance", "attach"],
    cli_positional: &[],
    mcp_tool: Some("instance_attach"),
    read_only: false,
    capability: None,
    input_schema: schema::<AttachInput>,
    output_schema: schema::<AttachOutput>,
};

static DETACH: OperationSpec = OperationSpec {
    name: "instance.detach",
    group: "instance",
    toolset: Toolset::Instances,
    summary: "Detach from a Live browser Instance; the browser keeps running",
    http: Http {
        method: Method::Post,
        path: "/v1/instances/{instance_id}/detach",
    },
    cli: &["instance", "detach"],
    cli_positional: &["instance_id"],
    mcp_tool: Some("instance_detach"),
    read_only: false,
    capability: None,
    input_schema: schema::<DetachInput>,
    output_schema: schema::<Instance>,
};

inventory::submit!(OpRegistration {
    spec: &ATTACH,
    handler: attach
});
inventory::submit!(OpRegistration {
    spec: &DETACH,
    handler: detach
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AttachInput {
    // http://host:port (its /json/version names the browser) or a ws:// browser endpoint.
    cdp_url: Option<String>,
    // chrome, chrome-beta, chromium, brave or edge: read from the browser's DevToolsActivePort.
    install_name: Option<String>,
    // extension: wait for the brr companion extension to connect (the user clicks "Allow and connect").
    via: Option<AttachVia>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AttachVia {
    Extension,
}

#[derive(Serialize, JsonSchema)]
struct AttachOutput {
    instance: Instance,
    tabs: Vec<Tab>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DetachInput {
    instance_id: brr_core::InstanceId,
}

fn attach(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: AttachInput = parse(input)?;
        // The error names a field the request spelled, not a fixed one.
        let first_sent = [
            ("cdp_url", input.cdp_url.is_some()),
            ("install_name", input.install_name.is_some()),
            ("via", input.via.is_some()),
        ]
        .into_iter()
        .find_map(|(field, sent)| sent.then_some(field))
        .unwrap_or("input");
        let (attached, tabs) = match (input.cdp_url, input.install_name, input.via) {
            (None, None, Some(AttachVia::Extension)) => {
                ctx.app
                    .relay
                    .wait_attached(&ctx.app, EXTENSION_WAIT)
                    .await?
            }
            (Some(url), None, None) => {
                let source = AttachSource::CdpUrl(url);

                ctx.app.instances.attach(&ctx.app, source).await?
            }
            (None, Some(name), None) => {
                let source = AttachSource::InstallName(name);

                ctx.app.instances.attach(&ctx.app, source).await?
            }
            _ => return Err(invalid(first_sent, ATTACH_CHOICE)),
        };
        let mut views = Vec::with_capacity(tabs.len());
        for tab in tabs {
            views.push(tab_view(&ctx.app, tab).await);
        }

        output(&AttachOutput {
            instance: instance_view(&ctx.app, attached).await?,
            tabs: views,
        })
    }
    .boxed()
}

fn detach(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: DetachInput = parse(input)?;

        ctx.app
            .leases
            .guard(
                &ctx.app,
                &ctx.client,
                &input.instance_id,
                None,
                DETACH.read_only,
            )
            .await?;
        let detached = ctx
            .app
            .instances
            .detach(&ctx.app, &input.instance_id)
            .await?;

        output(&instance_view(&ctx.app, detached).await?)
    }
    .boxed()
}
