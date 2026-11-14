use std::sync::Arc;

use brr_core::{
    Error, Http, InstanceId, Lease, Method, OperationSpec, Tab, TabCloseReason, TabId, TabState,
    Toolset, WaitUntil, schema,
};
use brr_store::{instance, tab};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    OpRegistration, ensure_live, invalid, output, page_url, parse,
    tool::{page_info, timeout},
};
use crate::{
    app::{App, Ctx},
    services::{events, instances::InstanceView},
};

pub(super) const DEFAULT_NAVIGATE_TIMEOUT_MS: u64 = 30_000;

static LIST: OperationSpec = OperationSpec {
    name: "tab.list",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "List open Tabs of an Instance (default: the Instance of your current Tab), or of every running Instance",
    http: Http {
        method: Method::Get,
        path: "/v1/instances/{instance_id}/tabs",
    },
    cli: &["tab", "list"],
    cli_positional: &[],
    mcp_tool: Some("tab_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static OPEN: OperationSpec = OperationSpec {
    name: "tab.open",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "Open a Tab in an Instance (default: the Instance of your current Tab) and lease it to you",
    http: Http {
        method: Method::Post,
        path: "/v1/instances/{instance_id}/tabs",
    },
    cli: &["tab", "open"],
    cli_positional: &[],
    mcp_tool: Some("tab_open"),
    read_only: false,
    capability: None,
    input_schema: schema::<OpenInput>,
    output_schema: schema::<OpenOutput>,
};

static GET: OperationSpec = OperationSpec {
    name: "tab.get",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "Show one Tab, including its Lease",
    http: Http {
        method: Method::Get,
        path: "/v1/tabs/{tab_id}",
    },
    cli: &["tab", "get"],
    cli_positional: &["tab_id"],
    mcp_tool: Some("tab_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<GetInput>,
    output_schema: schema::<Tab>,
};

static CLOSE: OperationSpec = OperationSpec {
    name: "tab.close",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "Close a Tab (default: your current Tab)",
    http: Http {
        method: Method::Delete,
        path: "/v1/tabs/{tab_id}",
    },
    cli: &["tab", "close"],
    cli_positional: &[],
    mcp_tool: Some("tab_close"),
    read_only: false,
    capability: None,
    input_schema: schema::<CloseInput>,
    output_schema: schema::<Tab>,
};

static SWITCH: OperationSpec = OperationSpec {
    name: "tab.switch",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "Make a Tab your current Tab and bring it to the front",
    http: Http {
        method: Method::Post,
        path: "/v1/tabs/{tab_id}/switch",
    },
    cli: &["tab", "switch"],
    cli_positional: &["tab_id"],
    mcp_tool: Some("tab_switch"),
    read_only: false,
    capability: None,
    input_schema: schema::<GetInput>,
    output_schema: schema::<Tab>,
};

static NAVIGATE: OperationSpec = OperationSpec {
    name: "tab.navigate",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "Navigate a Tab (default: your current Tab) to a URL and wait until it loads",
    http: Http {
        method: Method::Post,
        path: "/v1/tabs/{tab_id}/navigate",
    },
    cli: &["tab", "navigate"],
    cli_positional: &["url"],
    mcp_tool: Some("navigate"),
    read_only: false,
    capability: None,
    input_schema: schema::<NavigateInput>,
    output_schema: schema::<Tab>,
};

static PIN: OperationSpec = OperationSpec {
    name: "tab.pin",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "Pin a Tab so idle reaping never closes it",
    http: Http {
        method: Method::Post,
        path: "/v1/tabs/{tab_id}/pin",
    },
    cli: &["tab", "pin"],
    cli_positional: &["tab_id"],
    mcp_tool: Some("tab_pin"),
    read_only: false,
    capability: None,
    input_schema: schema::<GetInput>,
    output_schema: schema::<Tab>,
};

static UNPIN: OperationSpec = OperationSpec {
    name: "tab.unpin",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "Unpin a Tab so idle reaping may close it again",
    http: Http {
        method: Method::Post,
        path: "/v1/tabs/{tab_id}/unpin",
    },
    cli: &["tab", "unpin"],
    cli_positional: &["tab_id"],
    mcp_tool: Some("tab_unpin"),
    read_only: false,
    capability: None,
    input_schema: schema::<GetInput>,
    output_schema: schema::<Tab>,
};

inventory::submit!(OpRegistration {
    spec: &LIST,
    handler: list
});
inventory::submit!(OpRegistration {
    spec: &OPEN,
    handler: open
});
inventory::submit!(OpRegistration {
    spec: &GET,
    handler: get
});
inventory::submit!(OpRegistration {
    spec: &CLOSE,
    handler: close
});
inventory::submit!(OpRegistration {
    spec: &SWITCH,
    handler: switch
});
inventory::submit!(OpRegistration {
    spec: &NAVIGATE,
    handler: navigate
});
inventory::submit!(OpRegistration {
    spec: &PIN,
    handler: pin
});
inventory::submit!(OpRegistration {
    spec: &UNPIN,
    handler: unpin
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    instance_id: Option<InstanceId>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    tabs: Vec<Tab>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenInput {
    instance_id: Option<InstanceId>,
    url: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct OpenOutput {
    tab: Tab,
    lease: Lease,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetInput {
    tab_id: TabId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CloseInput {
    tab_id: Option<TabId>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NavigateInput {
    tab_id: Option<TabId>,
    url: String,
    wait_until: Option<WaitUntil>,
    timeout: Option<u64>,
}

// Live state (url, title, state) of an open Tab comes from its owner; the Lease is always current.
pub(super) async fn tab_view(app: &Arc<App>, stored: Tab) -> Tab {
    let view = app.instances.view(&stored.instance_id);

    tab_view_of(app, view.as_ref(), stored).await
}

async fn tab_view_of(app: &Arc<App>, view: Option<&InstanceView>, stored: Tab) -> Tab {
    let live = view.and_then(|view| view.tabs.iter().find(|t| t.id == stored.id).cloned());
    let mut tab = live.unwrap_or(stored);
    tab.lease = app.leases.tab_lease(&tab.id).await;

    tab
}

// The Tab's own title only follows navigations, and Chromium does not report a title a script
// sets later, or one that arrives after the load event; the page knows. A page at a dialog answers
// nothing, a closed Tab has no page: both keep the view they have.
async fn refresh_info(app: &Arc<App>, tab: &mut Tab) {
    if let Ok(page) = app.instances.page(&tab.instance_id, &tab.id).await
        && page.state().dialog().borrow().is_none()
        && let Ok((url, title)) = page_info(&page).await
    {
        app.instances
            .tab_info(&tab.instance_id, &tab.id, url.clone(), title.clone())
            .await;
        tab.url = url;
        tab.title = title;
    }
}

fn no_current_tab(field: &str) -> Error {
    invalid(field, &format!("no current tab; pass {field}"))
}

async fn current_tab(ctx: &Ctx) -> Option<TabId> {
    ctx.app.clients.current_tab(&ctx.client.client_id).await
}

async fn stored_tab(ctx: &Ctx, tab_id: TabId) -> Result<Tab, Error> {
    ctx.app.store.run(move |c| tab::get(c, &tab_id)).await
}

async fn current_instance(ctx: &Ctx) -> Result<Option<InstanceId>, Error> {
    match current_tab(ctx).await {
        Some(tab_id) => Ok(Some(stored_tab(ctx, tab_id).await?.instance_id)),
        None => Ok(None),
    }
}

// The open Tab a tool addresses: `tab_id`, else the caller's current Tab.
pub(super) async fn open_tab(ctx: &Ctx, tab_id: Option<TabId>) -> Result<Tab, Error> {
    let tab_id = match tab_id {
        Some(tab_id) => tab_id,
        None => current_tab(ctx)
            .await
            .ok_or_else(|| no_current_tab("tab_id"))?,
    };
    let stored = stored_tab(ctx, tab_id).await?;
    if stored.state == TabState::Closed {
        return Err(Error::NotFound {
            what: format!("open tab {tab_id}"),
        });
    }

    Ok(stored)
}

// Mutating ops take the lease through the guard; `read_only` is the op's own SPEC flag.
async fn guarded_tab(ctx: &Ctx, tab_id: Option<TabId>, read_only: bool) -> Result<Tab, Error> {
    let stored = open_tab(ctx, tab_id).await?;

    ctx.app
        .leases
        .guard(
            &ctx.app,
            &ctx.client,
            &stored.instance_id,
            Some(&stored.id),
            read_only,
        )
        .await?;

    Ok(stored)
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let instance_id = match input.instance_id {
            Some(instance_id) => Some(instance_id),
            None => current_instance(&ctx).await?,
        };

        let groups = ctx
            .app
            .store
            .run(move |c| {
                let instance_ids = match instance_id {
                    Some(instance_id) => {
                        instance::get(c, &instance_id)?;

                        vec![instance_id]
                    }
                    None => instance::non_final(c)?.into_iter().map(|i| i.id).collect(),
                };

                instance_ids
                    .into_iter()
                    .map(|id| Ok((id, tab::list(c, &id, false)?)))
                    .collect::<Result<Vec<_>, Error>>()
            })
            .await?;

        let mut tabs = Vec::new();
        for (instance_id, stored) in groups {
            let view = ctx.app.instances.view(&instance_id);
            for tab in stored {
                tabs.push(tab_view_of(&ctx.app, view.as_ref(), tab).await);
            }
        }

        output(&ListOutput {
            tabs,
            next_cursor: None,
        })
    }
    .boxed()
}

fn open(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: OpenInput = parse(input)?;
        input.url.as_deref().map(page_url).transpose()?;
        let instance_id = match input.instance_id {
            Some(instance_id) => instance_id,
            None => current_instance(&ctx)
                .await?
                .ok_or_else(|| no_current_tab("instance_id"))?,
        };

        ensure_live(&ctx, instance_id).await?;
        ctx.app
            .leases
            .guard(&ctx.app, &ctx.client, &instance_id, None, OPEN.read_only)
            .await?;
        let slot = ctx.app.admission.check_tab(&ctx.app)?;
        // Open and lease happen in one task: a dropped request must not leave an unleased Tab that
        // counts against max_tabs until the idle reaper closes it.
        let (app, client) = (Arc::clone(&ctx.app), ctx.client.clone());
        let (opened, lease) = tokio::spawn(events::keep_actor(async move {
            let _slot = slot;
            let opened = app.instances.open_tab(&instance_id, input.url).await?;
            let leased = app
                .leases
                .acquire_tab(&app, &client, &instance_id, &opened.id)
                .await;

            match leased {
                Ok(lease) => Ok((opened, lease)),
                Err(error) => {
                    let reason = TabCloseReason::Client;
                    if let Err(close) = app.instances.close_tab(&instance_id, &opened.id, reason).await {
                        tracing::warn!(%instance_id, tab_id = %opened.id, error = format!("{close:#}"), "unleased tab not closed");
                    }

                    Err(error)
                }
            }
        }))
        .await
        .map_err(crate::internal)??;
        ctx.app
            .clients
            .set_current_tab(&ctx.app, &ctx.client.client_id, Some(&opened.id))
            .await?;

        output(&OpenOutput {
            tab: tab_view(&ctx.app, opened).await,
            lease,
        })
    }
    .boxed()
}

fn get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: GetInput = parse(input)?;
        let stored = stored_tab(&ctx, input.tab_id).await?;
        let mut tab = tab_view(&ctx.app, stored).await;
        refresh_info(&ctx.app, &mut tab).await;

        output(&tab)
    }
    .boxed()
}

fn close(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: CloseInput = parse(input)?;
        let stored = guarded_tab(&ctx, input.tab_id, CLOSE.read_only).await?;

        let closed = ctx
            .app
            .instances
            .close_tab(&stored.instance_id, &stored.id, TabCloseReason::Client)
            .await?;
        if current_tab(&ctx).await == Some(closed.id) {
            ctx.app
                .clients
                .set_current_tab(&ctx.app, &ctx.client.client_id, None)
                .await?;
        }

        output(&tab_view(&ctx.app, closed).await)
    }
    .boxed()
}

fn switch(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: GetInput = parse(input)?;
        let stored = guarded_tab(&ctx, Some(input.tab_id), SWITCH.read_only).await?;

        ctx.app
            .instances
            .activate_tab(&stored.instance_id, &stored.id)
            .await?;
        ctx.app
            .clients
            .set_current_tab(&ctx.app, &ctx.client.client_id, Some(&stored.id))
            .await?;

        output(&tab_view(&ctx.app, stored).await)
    }
    .boxed()
}

fn navigate(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: NavigateInput = parse(input)?;
        page_url(&input.url)?;
        let stored = guarded_tab(&ctx, input.tab_id, NAVIGATE.read_only).await?;

        let timeout = timeout(input.timeout)?;
        let navigated = ctx
            .app
            .instances
            .navigate(
                &stored.instance_id,
                &stored.id,
                input.url,
                input.wait_until.unwrap_or(WaitUntil::Load),
                timeout,
            )
            .await?;
        ctx.app
            .clients
            .set_current_tab(&ctx.app, &ctx.client.client_id, Some(&stored.id))
            .await?;

        let mut tab = tab_view(&ctx.app, navigated).await;
        refresh_info(&ctx.app, &mut tab).await;

        output(&tab)
    }
    .boxed()
}

fn pin(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: GetInput = parse(input)?;

        set_pinned(&ctx, input.tab_id, true, PIN.read_only).await
    }
    .boxed()
}

fn unpin(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: GetInput = parse(input)?;

        set_pinned(&ctx, input.tab_id, false, UNPIN.read_only).await
    }
    .boxed()
}

async fn set_pinned(
    ctx: &Ctx,
    tab_id: TabId,
    pinned: bool,
    read_only: bool,
) -> Result<Value, Error> {
    let stored = guarded_tab(ctx, Some(tab_id), read_only).await?;
    let updated = ctx
        .app
        .instances
        .set_tab_pinned(&stored.instance_id, &stored.id, pinned)
        .await?;

    output(&tab_view(&ctx.app, updated).await)
}
