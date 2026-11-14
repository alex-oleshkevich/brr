use std::time::Duration;

use brr_core::{
    Error, InstanceId, ProfileId, Tab, TabId,
    capture::{DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS},
};
use brr_engine::page::{Page, Target};
use brr_store::{build, instance};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{
    invalid, output,
    tabs::{open_tab, tab_view},
};
use crate::{app::Ctx, internal, services::artifacts::ArtifactOwner};

const PAGE_INFO_EXPRESSION: &str = "[location.href, document.title]";

// The input of a Tab tool that takes nothing else.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabInput {
    pub tab_id: Option<TabId>,
}

pub struct TabCall {
    pub instance_id: InstanceId,
    pub tab_id: TabId,
    pub profile_id: Option<ProfileId>,
    pub page: Page,
}

// Registers one Tab tool: `POST /v1/tabs/{tab_id}/<tool>`, parsed into `input` and run by the
// async fn `run`.
macro_rules! tab_op {
    (
        $spec:ident {
            tool: $tool:literal,
            cli: $cli:literal,
            summary: $summary:literal,
            positional: $positional:expr,
            read_only: $read_only:expr,
            capability: $capability:expr,
            input: $input:ty,
            output: $output:ty,
            run: $run:path $(,)?
        }
    ) => {
        static $spec: brr_core::OperationSpec = brr_core::OperationSpec {
            name: concat!("tab.", $tool),
            group: "tab",
            toolset: brr_core::Toolset::Browser,
            summary: $summary,
            http: brr_core::Http {
                method: brr_core::Method::Post,
                path: concat!("/v1/tabs/{tab_id}/", $tool),
            },
            cli: &["tab", $cli],
            cli_positional: $positional,
            mcp_tool: Some($tool),
            read_only: $read_only,
            capability: $capability,
            input_schema: brr_core::schema::<$input>,
            output_schema: brr_core::schema::<$output>,
        };

        const _: () = {
            fn handler(
                ctx: crate::app::Ctx,
                input: serde_json::Value,
            ) -> futures::future::BoxFuture<'static, Result<serde_json::Value, brr_core::Error>>
            {
                futures::FutureExt::boxed(async move {
                    $run(ctx, crate::ops::parse::<$input>(input)?).await
                })
            }

            inventory::submit!(crate::ops::OpRegistration {
                spec: &$spec,
                handler
            });
        };
    };
}

pub(super) use tab_op;

// Capability before Lease: a tool the Build cannot run must not take a Lease.
pub async fn tab_call(
    ctx: &Ctx,
    tab_id: Option<TabId>,
    spec: &'static brr_core::OperationSpec,
) -> Result<TabCall, Error> {
    let stored = open_tab(ctx, tab_id).await?;
    let tab_id = stored.id;
    let instance_id = stored.instance_id;
    let (profile_id, capabilities) = ctx
        .app
        .store
        .run(move |c| {
            let row = instance::get(c, &instance_id)?;
            let capabilities = match row.build_id {
                Some(build_id) => build::get(c, &build_id)?.capabilities,
                None => Vec::new(),
            };

            Ok((row.profile_id, capabilities))
        })
        .await?;
    if let Some(capability) = spec.capability
        && !capabilities.contains(&capability)
    {
        return Err(Error::EngineUnsupported { capability });
    }

    ctx.app
        .leases
        .guard(
            &ctx.app,
            &ctx.client,
            &instance_id,
            Some(&tab_id),
            spec.read_only,
        )
        .await?;
    let page = ctx.app.instances.page(&instance_id, &tab_id).await?;

    Ok(TabCall {
        instance_id,
        tab_id,
        profile_id,
        page,
    })
}

impl TabCall {
    pub fn owner(&self) -> ArtifactOwner {
        ArtifactOwner {
            profile_id: self.profile_id,
            instance_id: Some(self.instance_id),
            tab_id: Some(self.tab_id),
        }
    }

    // Url and title as the page has them now: the Tab's own view only follows navigations.
    // A page stopped at a dialog answers nothing until the dialog is handled, so it keeps the
    // view it has.
    pub async fn tab(&self, ctx: &Ctx) -> Result<Tab, Error> {
        let stored = open_tab(ctx, Some(self.tab_id)).await?;
        let mut tab = tab_view(&ctx.app, stored).await;
        if self.page.state().dialog().borrow().is_some() {
            return Ok(tab);
        }

        if let Ok((url, title)) = self.info().await {
            ctx.app
                .instances
                .tab_info(&self.instance_id, &self.tab_id, url.clone(), title.clone())
                .await;
            tab.url = url;
            tab.title = title;
        }

        Ok(tab)
    }

    // Both from one read, so they belong to the same document.
    pub async fn info(&self) -> Result<(String, String), Error> {
        page_info(&self.page).await
    }

    pub async fn tab_out(&self, ctx: &Ctx) -> Result<Value, Error> {
        output(&self.tab(ctx).await?)
    }
}

pub async fn page_info(page: &Page) -> Result<(String, String), Error> {
    let info = page.evaluate_utility(PAGE_INFO_EXPRESSION).await?;

    match info {
        Value::Array(items) => match items.as_slice() {
            [Value::String(url), Value::String(title)] => Ok((url.clone(), title.clone())),
            _ => Err(internal("the page answered with an unexpected shape")),
        },
        _ => Err(internal("the page answered with an unexpected shape")),
    }
}

pub fn timeout(timeout_ms: Option<u64>) -> Result<Duration, Error> {
    let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    if timeout_ms == 0 || timeout_ms > MAX_TIMEOUT_MS {
        return Err(invalid(
            "timeout",
            &format!("must be within 1..={MAX_TIMEOUT_MS} ms"),
        ));
    }

    Ok(Duration::from_millis(timeout_ms))
}

// No ref and no selector means the focused element.
pub fn optional_target(
    r#ref: Option<String>,
    selector: Option<String>,
) -> Result<Option<Target>, Error> {
    if r#ref.is_none() && selector.is_none() {
        return Ok(None);
    }

    Target::from_input(r#ref, selector).map(Some)
}
