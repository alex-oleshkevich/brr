#![allow(clippy::unwrap_used)]

use crate::common;

use std::{sync::Arc, time::Duration};

use brr_core::{
    ClientKind, ConflictReason, Error, IdleOverrides, Instance, InstanceId, InstanceState, Tab,
    TabId, TabState,
};
use brr_daemon::{App, CallClient, ops, services::events::Published};
use brr_store::{instance, tab};
use serde_json::{Value, json};
use tokio::{sync::broadcast::Receiver, time};

use common::{
    KillOnDrop, add_profile, chrome_app, ctx, events_of, instance_row, scratch_app, settled,
    tab_row,
};

const PROFILE: &str = "p";
const PAGE: &str = "data:text/html,<button>b</button>";
const EVENT_WAIT: Duration = Duration::from_secs(10);

async fn call(
    app: &Arc<App>,
    client: &CallClient,
    name: &str,
    input: Value,
) -> Result<Value, Error> {
    let mut context = ctx(app);
    context.client = client.clone();

    ops::dispatch(context, ops::find(name).unwrap(), input).await
}

async fn client(app: &Arc<App>, kind: ClientKind, label: &str) -> CallClient {
    let created = app
        .clients
        .create(app, kind, label.to_string())
        .await
        .unwrap();

    app.clients.resolve(app, &created.id).await.unwrap()
}

async fn chrome_launch() -> Option<(tempfile::TempDir, Arc<App>, KillOnDrop)> {
    let (dir, app, build_id) = chrome_app(&[]).await?;
    add_profile(&app, build_id, PROFILE, false, IdleOverrides::default()).await;
    let browsers = KillOnDrop::new(&app, Vec::new());

    Some((dir, app, browsers))
}

async fn launch(app: &Arc<App>, browsers: &KillOnDrop, client: &CallClient) -> (InstanceId, TabId) {
    let launched = call(
        app,
        client,
        "instance.launch",
        json!({"profile": PROFILE, "url": PAGE}),
    )
    .await
    .unwrap();
    let instance: Instance = serde_json::from_value(launched["instance"].clone()).unwrap();
    browsers.track(instance.id);
    let tab: Tab = serde_json::from_value(launched["tab"].clone()).unwrap();

    (instance.id, tab.id)
}

async fn stop(app: &Arc<App>, client: &CallClient, instance_id: InstanceId) {
    call(
        app,
        client,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await
    .unwrap();
}

async fn get_tab(app: &Arc<App>, client: &CallClient, tab_id: TabId) -> Value {
    call(app, client, "tab.get", json!({"tab_id": tab_id}))
        .await
        .unwrap()
}

// The events a subscriber saw so far, as (kind, data).
fn seen(events: &mut Receiver<Arc<Published>>) -> Vec<(String, Value)> {
    let mut seen = Vec::new();
    while let Ok(published) = events.try_recv() {
        seen.push((published.event.kind.clone(), published.event.data.clone()));
    }

    seen
}

async fn tab_state(app: &Arc<App>, client: &CallClient, tab_id: TabId) -> Value {
    get_tab(app, client, tab_id).await["state"].clone()
}

#[tokio::test]
async fn take_over_hand_back() {
    let Some((_dir, app, browsers)) = chrome_launch().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let u = client(&app, ClientKind::Ui, "ui").await;
    let (instance_id, tab_id) = launch(&app, &browsers, &a).await;
    let mut events = app.events.subscribe();

    let taken = call(&app, &u, "tab.take_over", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    assert_eq!(taken["lease"]["client_id"], u.client_id.to_string());

    let transferred = seen(&mut events)
        .into_iter()
        .find(|(kind, _)| kind == "lease.transferred")
        .unwrap()
        .1;
    assert_eq!(transferred["from_client_id"], a.client_id.to_string());
    assert_eq!(transferred["to_client_id"], u.client_id.to_string());
    assert_eq!(transferred["instance_id"], instance_id.to_string());
    let lease = &get_tab(&app, &a, tab_id).await["lease"];
    assert_eq!(lease["client_id"], u.client_id.to_string());
    assert_eq!(lease["transferred_from_client_id"], a.client_id.to_string());

    let blocked = call(
        &app,
        &a,
        "tab.click",
        json!({"tab_id": tab_id, "selector": "button"}),
    )
    .await;
    assert_eq!(
        blocked,
        Err(Error::LeaseHeld {
            client_id: u.client_id.clone()
        })
    );

    let back = call(&app, &u, "tab.hand_back", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    assert_eq!(back["lease"]["client_id"], a.client_id.to_string());
    let lease = &get_tab(&app, &a, tab_id).await["lease"];
    assert_eq!(lease["client_id"], a.client_id.to_string());
    assert_eq!(lease["transferred_from_client_id"], Value::Null);
    let handed = seen(&mut events)
        .into_iter()
        .find(|(kind, _)| kind == "lease.transferred")
        .unwrap()
        .1;
    assert_eq!(handed["from_client_id"], u.client_id.to_string());
    assert_eq!(handed["to_client_id"], a.client_id.to_string());

    let again = call(&app, &u, "tab.hand_back", json!({"tab_id": tab_id})).await;
    assert!(matches!(again, Err(Error::InvalidArgument { field, .. }) if field == "tab_id"));

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn hand_back_client_gone() {
    let (_dir, app) = scratch_app().await;
    let row = instance_row(InstanceState::Running, None);
    let seeded = tab_row(row.id, TabState::Ready);
    let (instance_id, tab_id) = (row.id, seeded.id);
    app.store
        .run(move |c| {
            instance::insert(c, &row)?;
            tab::insert(c, &seeded)
        })
        .await
        .unwrap();
    let implicit = app.clients.implicit(&app, ClientKind::Mcp);
    app.leases
        .acquire_tab(&app, &implicit.client, &instance_id, &tab_id)
        .await
        .unwrap();
    let u = client(&app, ClientKind::Ui, "ui").await;

    call(&app, &u, "tab.take_over", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    let back = call(&app, &u, "tab.hand_back", json!({"tab_id": tab_id})).await;

    assert!(matches!(
        back,
        Err(Error::Conflict {
            reason: ConflictReason::ClientGone,
            ..
        })
    ));
    let lease = &get_tab(&app, &u, tab_id).await["lease"];
    assert_eq!(lease["client_id"], u.client_id.to_string());
    assert_eq!(
        lease["transferred_from_client_id"],
        implicit.client.client_id.to_string()
    );
}

#[tokio::test]
async fn await_then_done() {
    let Some((_dir, app, browsers)) = chrome_launch().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let u = client(&app, ClientKind::Ui, "ui").await;
    let (instance_id, tab_id) = launch(&app, &browsers, &a).await;
    let mut events = app.events.subscribe();

    let waiting = tokio::spawn({
        let (app, a) = (Arc::clone(&app), a.clone());
        async move {
            call(
                &app,
                &a,
                "tab.await_human",
                json!({"tab_id": tab_id, "reason": "login", "timeout_ms": 60000}),
            )
            .await
        }
    });
    settled(async || tab_state(&app, &a, tab_id).await == "awaiting_human").await;
    assert!(!waiting.is_finished());

    let done = call(&app, &u, "tab.human_done", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    assert_eq!(done["state"], "ready");
    let answered = time::timeout(EVENT_WAIT, waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(answered["state"], "ready");

    let changes: Vec<(String, String)> = seen(&mut events)
        .into_iter()
        .filter(|(kind, _)| kind == "tab.state_changed")
        .map(|(_, data)| {
            (
                data["to"].as_str().unwrap().to_owned(),
                data["trigger"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        changes,
        [
            (
                "awaiting_human".to_owned(),
                "captcha_needs_human".to_owned()
            ),
            ("ready".to_owned(), "human_done".to_owned()),
        ]
    );
    let stored = events_of(&app, "tab.awaiting_human", instance_id).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["reason"], "login");
    assert_eq!(stored[0]["tab_id"], tab_id.to_string());

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn human_done_hands_the_lease_back() {
    let Some((_dir, app, browsers)) = chrome_launch().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let u = client(&app, ClientKind::Ui, "ui").await;
    let (instance_id, tab_id) = launch(&app, &browsers, &a).await;

    let waiting = tokio::spawn({
        let (app, a) = (Arc::clone(&app), a.clone());
        async move {
            call(
                &app,
                &a,
                "tab.await_human",
                json!({"tab_id": tab_id, "reason": "captcha"}),
            )
            .await
        }
    });
    settled(async || tab_state(&app, &a, tab_id).await == "awaiting_human").await;
    call(&app, &u, "tab.take_over", json!({"tab_id": tab_id}))
        .await
        .unwrap();

    call(&app, &u, "tab.human_done", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    time::timeout(EVENT_WAIT, waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let lease = &get_tab(&app, &a, tab_id).await["lease"];
    assert_eq!(lease["client_id"], a.client_id.to_string());
    assert_eq!(lease["transferred_from_client_id"], Value::Null);

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn the_ui_navigates_a_waiting_tab_without_taking_it_over() {
    let Some((_dir, app, browsers)) = chrome_launch().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let u = client(&app, ClientKind::Ui, "ui").await;
    let (instance_id, tab_id) = launch(&app, &browsers, &a).await;

    let refused = call(&app, &u, "tab.reload", json!({"tab_id": tab_id})).await;
    assert!(matches!(refused, Err(Error::LeaseHeld { .. })));

    let waiting = tokio::spawn({
        let (app, a) = (Arc::clone(&app), a.clone());
        async move {
            call(
                &app,
                &a,
                "tab.await_human",
                json!({"tab_id": tab_id, "reason": "login"}),
            )
            .await
        }
    });
    settled(async || tab_state(&app, &a, tab_id).await == "awaiting_human").await;

    call(&app, &u, "tab.reload", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    let lease = &get_tab(&app, &a, tab_id).await["lease"];
    assert_eq!(lease["client_id"], a.client_id.to_string());

    call(&app, &u, "tab.human_done", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    time::timeout(EVENT_WAIT, waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn await_timeout() {
    let Some((_dir, app, browsers)) = chrome_launch().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let (instance_id, tab_id) = launch(&app, &browsers, &a).await;
    let mut events = app.events.subscribe();

    let started = time::Instant::now();
    let timed_out = call(
        &app,
        &a,
        "tab.await_human",
        json!({"tab_id": tab_id, "reason": "dialog", "timeout_ms": 500}),
    )
    .await;
    assert_eq!(timed_out, Err(Error::Timeout { after_ms: 500 }));
    assert!(started.elapsed() >= Duration::from_millis(500));
    assert!(started.elapsed() < Duration::from_secs(5));

    assert_eq!(tab_state(&app, &a, tab_id).await, "ready");
    let triggers: Vec<String> = seen(&mut events)
        .into_iter()
        .filter(|(kind, _)| kind == "tab.state_changed")
        .map(|(_, data)| data["trigger"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(triggers, ["dialog_blocking", "human_timeout"]);

    let u = client(&app, ClientKind::Ui, "ui").await;
    let done = call(&app, &u, "tab.human_done", json!({"tab_id": tab_id})).await;
    assert!(matches!(
        done,
        Err(Error::InvalidTransition { ref from, ref to }) if from == "ready" && to == "ready"
    ));

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn closing_the_tab_ends_the_wait() {
    let Some((_dir, app, browsers)) = chrome_launch().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let (instance_id, _) = launch(&app, &browsers, &a).await;
    let second = call(&app, &a, "tab.open", json!({"instance_id": instance_id}))
        .await
        .unwrap();
    let second_id: TabId = serde_json::from_value(second["tab"]["id"].clone()).unwrap();

    let waiting = tokio::spawn({
        let (app, a) = (Arc::clone(&app), a.clone());
        async move {
            call(
                &app,
                &a,
                "tab.await_human",
                json!({"tab_id": second_id, "reason": "login", "timeout_ms": 60000}),
            )
            .await
        }
    });
    settled(async || tab_state(&app, &a, second_id).await == "awaiting_human").await;

    call(&app, &a, "tab.close", json!({"tab_id": second_id}))
        .await
        .unwrap();
    let ended = time::timeout(EVENT_WAIT, waiting).await.unwrap().unwrap();
    assert!(matches!(ended, Err(Error::NotFound { .. })));

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn stopping_the_instance_ends_the_wait() {
    let Some((_dir, app, browsers)) = chrome_launch().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let (instance_id, tab_id) = launch(&app, &browsers, &a).await;

    let waiting = tokio::spawn({
        let (app, a) = (Arc::clone(&app), a.clone());
        async move {
            call(
                &app,
                &a,
                "tab.await_human",
                json!({"tab_id": tab_id, "reason": "captcha", "timeout_ms": 60000}),
            )
            .await
        }
    });
    settled(async || tab_state(&app, &a, tab_id).await == "awaiting_human").await;

    stop(&app, &a, instance_id).await;
    let ended = time::timeout(EVENT_WAIT, waiting).await.unwrap().unwrap();
    assert!(matches!(ended, Err(Error::NotFound { .. })));
}

#[tokio::test]
async fn only_the_ui_takes_over_hands_back_or_finishes() {
    let (_dir, app) = scratch_app().await;
    let agent = client(&app, ClientKind::Rest, "agent").await;

    for op in ["tab.take_over", "tab.hand_back", "tab.human_done"] {
        let refused = call(&app, &agent, op, json!({"tab_id": TabId::new()})).await;

        assert!(
            matches!(refused, Err(Error::InvalidArgument { ref field, .. }) if field == "client"),
            "{op}: {refused:?}"
        );
    }
}
