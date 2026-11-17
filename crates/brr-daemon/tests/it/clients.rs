#![allow(clippy::unwrap_used)]

use crate::common;

use std::sync::Arc;

use brr_core::{
    Client, ClientEndReason, ClientKind, InstanceId, Lease, LeaseId, ReleaseReason, TabId, now_ms,
};
use brr_daemon::{App, ops};
use brr_store::lease;
use serde_json::{Value, json};

use common::{ctx, scratch_app};

const AGENT: &str = "agent";
const GONE: &str = "gone";

fn lease_row(app_client: &Client, released: bool) -> Lease {
    Lease {
        id: LeaseId::new(),
        client_id: app_client.id.clone(),
        client_kind: app_client.kind,
        client_label: app_client.label.clone(),
        instance_id: InstanceId::new(),
        tab_id: Some(TabId::new()),
        exclusive: false,
        acquired_at: now_ms(),
        released_at: released.then(now_ms),
        release_reason: released.then_some(ReleaseReason::Released),
    }
}

async fn list(app: &Arc<App>) -> Vec<Value> {
    let listed = ops::dispatch(ctx(app), ops::find("client.list").unwrap(), json!({}))
        .await
        .unwrap();

    listed["clients"].as_array().unwrap().clone()
}

#[tokio::test]
async fn list_shows_active_clients_with_their_lease_count() {
    let (_dir, app) = scratch_app().await;
    let agent = app
        .clients
        .create(&app, ClientKind::Mcp, AGENT.to_string())
        .await
        .unwrap();
    let gone = app
        .clients
        .create(&app, ClientKind::Rest, GONE.to_string())
        .await
        .unwrap();
    let rows = [
        lease_row(&agent, false),
        lease_row(&agent, false),
        lease_row(&agent, true),
    ];
    app.store
        .run(move |c| rows.iter().try_for_each(|row| lease::insert(c, row)))
        .await
        .unwrap();
    app.clients
        .end(&app, &gone.id, ClientEndReason::Ended)
        .await
        .unwrap();

    let clients = list(&app).await;

    assert_eq!(clients.len(), 1, "{clients:?}");
    assert_eq!(clients[0]["id"], agent.id.to_string());
    assert_eq!(clients[0]["kind"], "mcp");
    assert_eq!(clients[0]["label"], AGENT);
    assert_eq!(clients[0]["created_at"], agent.created_at);
    assert_eq!(clients[0]["ended_at"], Value::Null);
    assert_eq!(clients[0]["lease_count"], 2);
}
