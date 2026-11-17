#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::sync::Arc;

use brr_core::{
    BuildChoice, BuildId, ClientEndReason, ClientKind, Display, Error, Instance, InstanceId,
    InstanceState, Profile, ProfileId, ProfileKind, ProfileSettings, Tab, TabId, TabState, now_ms,
};
use brr_daemon::{App, CallClient, ops};
use brr_store::{event, instance, profile, tab};
use serde_json::{Value, json};

use common::{KillOnDrop, boot, chrome, ctx, instance_row, register_build, scratch_app, tab_row};

const PAGE_A: &str = "data:text/html,a";
const PAGE_B: &str = "data:text/html,b";

async fn launch(browsers: &KillOnDrop, app: &Arc<App>, client: &CallClient, input: Value) -> Value {
    let launched = call(app, client, "instance.launch", input).await.unwrap();
    browsers.track(field(&launched["instance"], "id"));

    launched
}

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

fn field<T: serde::de::DeserializeOwned>(value: &Value, name: &str) -> T {
    serde_json::from_value(value[name].clone()).unwrap()
}

async fn rest_client(app: &Arc<App>, label: &str) -> CallClient {
    let created = app
        .clients
        .create(app, ClientKind::Rest, label.to_string())
        .await
        .unwrap();

    app.clients.resolve(app, &created.id).await.unwrap()
}

async fn add_ephemeral_profile(app: &Arc<App>, build_id: BuildId, name: &str) -> Profile {
    let now = now_ms();
    let created = Profile {
        id: ProfileId::new(),
        name: name.to_owned(),
        kind: ProfileKind::Ephemeral,
        template_id: None,
        template_name: None,
        settings: ProfileSettings {
            build: BuildChoice::BuildId(build_id),
            display: Display::Headless,
            ..ProfileSettings::default()
        },
        pinned: false,
        notes: String::new(),
        tags: Vec::new(),
        data_dir: None,
        disk_quota_bytes: None,
        backup_schedule: None,
        backup_keep: 0,
        created_at: now,
        updated_at: now,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    };
    let row = created.clone();
    app.store
        .run(move |c| profile::insert(c, &row))
        .await
        .unwrap();

    created
}

async fn chrome_app() -> Option<(tempfile::TempDir, Arc<App>, KillOnDrop)> {
    let chrome = chrome()?;
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[])).await.unwrap();
    let build_id = register_build(&app, &chrome).await;
    add_ephemeral_profile(&app, build_id, "p").await;
    let guard = KillOnDrop::new(&app, Vec::new());

    Some((dir, app, guard))
}

#[tokio::test]
async fn stop_stopped_is_invalid() {
    let (_dir, app) = scratch_app().await;
    let stopped = instance_row(InstanceState::Stopped, None);
    let instance_id = stopped.id;
    app.store
        .run(move |c| instance::insert(c, &stopped))
        .await
        .unwrap();
    let client = rest_client(&app, "a").await;
    let before = app
        .store
        .run(move |c| Ok((instance::get(c, &instance_id)?, event::max_seq(c)?)))
        .await
        .unwrap();

    let result = call(
        &app,
        &client,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await;

    assert_eq!(
        result,
        Err(Error::InvalidTransition {
            from: "stopped".to_string(),
            to: "stopping".to_string()
        })
    );
    let after = app
        .store
        .run(move |c| Ok((instance::get(c, &instance_id)?, event::max_seq(c)?)))
        .await
        .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn validation_and_missing_targets() {
    let (_dir, app) = scratch_app().await;
    let client = ctx(&app).client;

    let priority = call(
        &app,
        &client,
        "instance.launch",
        json!({"profile": "p", "priority": 10}),
    )
    .await;
    assert!(matches!(priority, Err(Error::InvalidArgument { field, .. }) if field == "priority"));

    let state = call(
        &app,
        &client,
        "instance.list",
        json!({"state": "running,bogus"}),
    )
    .await;
    assert!(matches!(state, Err(Error::InvalidArgument { field, .. }) if field == "state"));

    let no_instance = call(&app, &client, "tab.open", json!({})).await;
    assert!(
        matches!(no_instance, Err(Error::InvalidArgument { field, .. }) if field == "instance_id")
    );

    let no_tab = call(&app, &client, "tab.close", json!({})).await;
    assert!(matches!(no_tab, Err(Error::InvalidArgument { field, .. }) if field == "tab_id"));

    let unknown = call(&app, &client, "tab.get", json!({"tab_id": TabId::new()})).await;
    assert!(matches!(unknown, Err(Error::NotFound { .. })));
}

#[tokio::test]
async fn closed_tab_is_not_found_for_mutating_ops_only() {
    let (_dir, app) = scratch_app().await;
    let running = instance_row(InstanceState::Running, None);
    let closed = tab_row(running.id, TabState::Closed);
    let tab_id = closed.id;
    app.store
        .run(move |c| {
            instance::insert(c, &running)?;
            tab::insert(c, &closed)
        })
        .await
        .unwrap();
    let client = ctx(&app).client;

    let navigate = call(
        &app,
        &client,
        "tab.navigate",
        json!({"tab_id": tab_id, "url": PAGE_A}),
    )
    .await;
    assert_eq!(
        navigate,
        Err(Error::NotFound {
            what: format!("open tab {tab_id}")
        })
    );

    let got = call(&app, &client, "tab.get", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    assert_eq!(got["state"], "closed");
}

#[tokio::test]
async fn lease_rules() {
    let Some((_dir, app, browsers)) = chrome_app().await else {
        return;
    };
    let (a, b) = (rest_client(&app, "a").await, rest_client(&app, "b").await);

    let launched = launch(&browsers, &app, &a, json!({"profile": "p", "url": PAGE_A})).await;
    let instance: Instance = field(&launched, "instance");
    let first: Tab = field(&launched, "tab");
    assert_eq!(launched["lease"]["client_id"], json!(a.client_id));
    assert_eq!(first.lease.as_ref().unwrap().client_id, a.client_id);
    assert_eq!(instance.profile_name.as_deref(), Some("p"));
    assert_eq!(instance.build.as_ref().unwrap().label, "Chrome 999.0.0.0");
    assert_eq!(instance.tab_count, 1);
    assert_eq!(instance.lease_count, 1);

    let blocked = call(
        &app,
        &b,
        "tab.navigate",
        json!({"tab_id": first.id, "url": PAGE_B}),
    )
    .await;
    assert_eq!(
        blocked,
        Err(Error::LeaseHeld {
            client_id: a.client_id.clone()
        })
    );
    let got = call(&app, &b, "tab.get", json!({"tab_id": first.id}))
        .await
        .unwrap();
    assert_eq!(got["lease"]["client_id"], json!(a.client_id));
    let listed = call(&app, &b, "tab.list", json!({"instance_id": instance.id}))
        .await
        .unwrap();
    assert_eq!(listed["tabs"].as_array().unwrap().len(), 1);

    let navigated = call(
        &app,
        &a,
        "tab.navigate",
        json!({"tab_id": first.id, "url": PAGE_B}),
    )
    .await
    .unwrap();
    assert_eq!(navigated["url"], PAGE_B);

    call(
        &app,
        &a,
        "instance.stop",
        json!({"instance_id": instance.id}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn exclusive_blocks_open() {
    let Some((_dir, app, browsers)) = chrome_app().await else {
        return;
    };
    let a = app
        .clients
        .create(&app, ClientKind::Rest, "a".to_string())
        .await
        .unwrap();
    let a_call = app.clients.resolve(&app, &a.id).await.unwrap();
    let b = rest_client(&app, "b").await;

    let launched = launch(
        &browsers,
        &app,
        &a_call,
        json!({"profile": "p", "exclusive": true}),
    )
    .await;
    let instance_id: InstanceId = field(&launched["instance"], "id");
    assert_eq!(launched["instance_lease"]["exclusive"], true);

    let blocked = call(&app, &b, "tab.open", json!({"instance_id": instance_id})).await;
    assert_eq!(
        blocked,
        Err(Error::LeaseHeld {
            client_id: a_call.client_id.clone()
        })
    );
    let stop = call(
        &app,
        &b,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await;
    assert!(matches!(stop, Err(Error::LeaseHeld { .. })));

    app.clients
        .end(&app, &a.id, ClientEndReason::Ended)
        .await
        .unwrap();

    let opened = call(&app, &b, "tab.open", json!({"instance_id": instance_id}))
        .await
        .unwrap();
    assert_eq!(opened["lease"]["client_id"], json!(b.client_id));
    call(
        &app,
        &b,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn relaunch_new_id() {
    let Some((_dir, app, browsers)) = chrome_app().await else {
        return;
    };
    let client = ctx(&app).client;

    let first = launch(&browsers, &app, &client, json!({"profile": "p"})).await;
    let first_id: InstanceId = field(&first["instance"], "id");
    let stopped = call(
        &app,
        &client,
        "instance.stop",
        json!({"instance_id": first_id}),
    )
    .await
    .unwrap();
    assert_eq!(stopped["state"], "stopped");

    let second = launch(&browsers, &app, &client, json!({"profile": "p"})).await;
    let second_id: InstanceId = field(&second["instance"], "id");
    assert_ne!(first_id, second_id);

    let running = call(
        &app,
        &client,
        "instance.list",
        json!({"state": "running,idle"}),
    )
    .await
    .unwrap();
    assert_eq!(running["instances"].as_array().unwrap().len(), 1);
    call(
        &app,
        &client,
        "instance.stop",
        json!({"instance_id": second_id}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn current_tab_default() {
    let Some((_dir, app, browsers)) = chrome_app().await else {
        return;
    };
    let a = rest_client(&app, "a").await;

    let launched = launch(&browsers, &app, &a, json!({"profile": "p"})).await;
    let instance_id: InstanceId = field(&launched["instance"], "id");
    let first: Tab = field(&launched, "tab");
    let opened = call(&app, &a, "tab.open", json!({})).await.unwrap();
    let second: Tab = field(&opened, "tab");
    assert_eq!(second.instance_id, instance_id);
    assert_eq!(app.clients.current_tab(&a.client_id).await, Some(second.id));

    call(&app, &a, "tab.switch", json!({"tab_id": first.id}))
        .await
        .unwrap();
    assert_eq!(app.clients.current_tab(&a.client_id).await, Some(first.id));

    let navigated = call(&app, &a, "tab.navigate", json!({"url": PAGE_A}))
        .await
        .unwrap();
    assert_eq!(navigated["id"], json!(first.id));
    assert_eq!(navigated["url"], PAGE_A);

    let closed = call(&app, &a, "tab.close", json!({})).await.unwrap();
    assert_eq!(closed["id"], json!(first.id));
    assert_eq!(app.clients.current_tab(&a.client_id).await, None);

    let implicit = ctx(&app).client;
    let no_tab = call(&app, &implicit, "tab.close", json!({})).await;
    assert!(matches!(no_tab, Err(Error::InvalidArgument { field, .. }) if field == "tab_id"));

    call(
        &app,
        &a,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn failed_launch_step_stops_the_instance() {
    let Some((_dir, app, browsers)) = chrome_app().await else {
        return;
    };
    let client = ctx(&app).client;

    let failed = call(
        &app,
        &client,
        "instance.launch",
        json!({"profile": "p", "url": "not a url"}),
    )
    .await;

    assert!(failed.is_err());
    let running = call(
        &app,
        &client,
        "instance.list",
        json!({"state": "running,idle,starting,stopping"}),
    )
    .await
    .unwrap();
    assert!(running["instances"].as_array().unwrap().is_empty());
    drop(browsers);
}

#[tokio::test]
async fn tab_list_checks_the_instance() {
    let (_dir, app) = scratch_app().await;
    let client = ctx(&app).client;
    let stopped = instance_row(InstanceState::Stopped, None);
    let instance_id = stopped.id;
    app.store
        .run(move |c| instance::insert(c, &stopped))
        .await
        .unwrap();

    let unknown = call(
        &app,
        &client,
        "tab.list",
        json!({"instance_id": InstanceId::new()}),
    )
    .await;
    assert!(matches!(unknown, Err(Error::NotFound { .. })));

    let listed = call(
        &app,
        &client,
        "tab.list",
        json!({"instance_id": instance_id}),
    )
    .await
    .unwrap();
    assert!(listed["tabs"].as_array().unwrap().is_empty());
}
