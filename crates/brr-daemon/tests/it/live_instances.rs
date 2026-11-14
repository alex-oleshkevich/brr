#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use brr_core::{ClientKind, Error, Instance, InstanceState, Tab};
use brr_daemon::{App, CallClient, ops};
use brr_store::instance;
use serde_json::{Value, json};
use tokio::time;

use common::{chrome, ctx, row, scratch_app, triggers, wait_for};

const PAGE_ONE: &str = "data:text/html,<title>one</title>one";
const PAGE_TWO: &str = "data:text/html,<title>two</title>two";
const NAVIGATED: &str = "data:text/html,<title>moved</title>moved";
const PORT_FILE_TIMEOUT: Duration = Duration::from_secs(20);
const PORT_FILE_POLL: Duration = Duration::from_millis(50);
const LOST_WITHIN: Duration = Duration::from_secs(5);
const OPEN_ANSWERS_WITHIN: Duration = Duration::from_secs(15);

// A browser the test owns, started like a user would: remote debugging on, in the install
// directory `chromium` under the app's config root.
struct Browser {
    child: Child,
    port: u16,
}

impl Browser {
    async fn start(config_root: &Path) -> Option<Browser> {
        let chrome = chrome()?;
        let user_data_dir = config_root.join("chromium");
        let child = Command::new(chrome)
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", user_data_dir.display()))
            .args(["--headless", "--no-first-run", "--no-default-browser-check"])
            .arg(PAGE_ONE)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut browser = Browser { child, port: 0 };

        let url = time::timeout(PORT_FILE_TIMEOUT, async {
            loop {
                if let Ok(url) = brr_engine::devtools_active_port(config_root, "chromium") {
                    return url;
                }
                time::sleep(PORT_FILE_POLL).await;
            }
        })
        .await
        .unwrap();
        browser.port = url
            .strip_prefix("ws://127.0.0.1:")
            .and_then(|rest| rest.split('/').next())
            .unwrap()
            .parse()
            .unwrap();

        // Headless Chromium takes one start URL; the second page comes from a short connection.
        let (cdp, _events) = brr_cdp::connect_ws(&url).await.unwrap();
        cdp.create_target(PAGE_TWO, false).await.unwrap();

        Some(browser)
    }

    fn cdp_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn alive(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn client(app: &Arc<App>, label: &str) -> CallClient {
    let created = app
        .clients
        .create(app, ClientKind::Rest, label.to_owned())
        .await
        .unwrap();

    app.clients.resolve(app, &created.id).await.unwrap()
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

async fn attach(app: &Arc<App>, client: &CallClient, input: Value) -> (Instance, Vec<Tab>) {
    let attached = call(app, client, "instance.attach", input).await.unwrap();

    (
        serde_json::from_value(attached["instance"].clone()).unwrap(),
        serde_json::from_value(attached["tabs"].clone()).unwrap(),
    )
}

#[tokio::test]
async fn validation() {
    let (_dir, app) = scratch_app().await;
    let rest = client(&app, "rest").await;

    for (input, field) in [
        (json!({}), "input"),
        (
            json!({"cdp_url": "http://127.0.0.1:1", "install_name": "chrome"}),
            "cdp_url",
        ),
        (
            json!({"install_name": "chrome", "via": "extension"}),
            "install_name",
        ),
    ] {
        let error = call(&app, &rest, "instance.attach", input)
            .await
            .unwrap_err();

        assert_eq!(
            error,
            Error::InvalidArgument {
                field: field.to_owned(),
                message: "pass exactly one of cdp_url, install_name or via".to_owned(),
            }
        );
    }

    let ids = app.store.run(|c| instance::all_ids(c)).await.unwrap();
    assert!(ids.is_empty());
}

#[tokio::test]
async fn attach_cdp_url() {
    let (dir, app) = scratch_app().await;
    let Some(browser) = Browser::start(dir.path()).await else {
        return;
    };
    let rest = client(&app, "rest").await;

    let (instance, tabs) = attach(&app, &rest, json!({"cdp_url": browser.cdp_url()})).await;

    assert_eq!(instance.state, InstanceState::Attached);
    assert_eq!(serde_json::to_value(instance.kind).unwrap(), json!("live"));
    assert_eq!(instance.profile_id, None);
    assert_eq!(tabs.len(), 2);
    assert!(tabs.iter().all(|tab| !tab.opened_by_brr));
    assert_eq!(triggers(&app, instance.id).await, ["attach", "connected"]);

    call(
        &app,
        &rest,
        "instance.detach",
        json!({"instance_id": instance.id}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn attach_install_name() {
    let (dir, app) = scratch_app().await;
    let rest = client(&app, "rest").await;

    let error = call(
        &app,
        &rest,
        "instance.attach",
        json!({"install_name": "chromium"}),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&error, Error::DependencyMissing { name, .. } if name == "DevToolsActivePort"),
        "{error:?}"
    );
    let ids = app.store.run(|c| instance::all_ids(c)).await.unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(row(&app, ids[0]).await.state, InstanceState::Failed);

    let Some(_browser) = Browser::start(dir.path()).await else {
        return;
    };
    let (instance, tabs) = attach(&app, &rest, json!({"install_name": "chromium"})).await;

    assert_eq!(instance.state, InstanceState::Attached);
    assert_eq!(tabs.len(), 2);
}

#[tokio::test]
async fn leases_and_close_rules() {
    let (dir, app) = scratch_app().await;
    let Some(_browser) = Browser::start(dir.path()).await else {
        return;
    };
    let (a, b) = (client(&app, "a").await, client(&app, "b").await);
    let (instance, tabs) = attach(&app, &a, json!({"install_name": "chromium"})).await;
    let user_tab = &tabs[0];

    call(
        &app,
        &a,
        "tab.navigate",
        json!({"tab_id": user_tab.id, "url": NAVIGATED}),
    )
    .await
    .unwrap();
    let held = call(
        &app,
        &b,
        "tab.navigate",
        json!({"tab_id": user_tab.id, "url": PAGE_TWO}),
    )
    .await
    .unwrap_err();
    assert!(matches!(held, Error::LeaseHeld { .. }), "{held:?}");

    let refused = call(&app, &a, "tab.close", json!({"tab_id": user_tab.id}))
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        Error::InvalidArgument {
            field: "tab_id".to_owned(),
            message: "brr closes only Tabs it opened in a Live browser".to_owned(),
        }
    );

    let opened = call(&app, &a, "tab.open", json!({"instance_id": instance.id}))
        .await
        .unwrap();
    let own: Tab = serde_json::from_value(opened["tab"].clone()).unwrap();
    assert!(own.opened_by_brr);
    call(&app, &a, "tab.close", json!({"tab_id": own.id}))
        .await
        .unwrap();
}

#[tokio::test]
async fn never_kills() {
    let (dir, app) = scratch_app().await;
    let Some(mut browser) = Browser::start(dir.path()).await else {
        return;
    };
    let rest = client(&app, "rest").await;

    // Detach leaves the browser and its pages.
    let (first, _) = attach(&app, &rest, json!({"cdp_url": browser.cdp_url()})).await;
    let detached = call(
        &app,
        &rest,
        "instance.detach",
        json!({"instance_id": first.id}),
    )
    .await
    .unwrap();
    assert_eq!(detached["state"], "detached");
    assert!(browser.alive());

    // A Live Instance does not count toward the Instance limit.
    app.set_settings([("limits.max_instances".to_owned(), json!(1))].into())
        .await
        .unwrap();
    let (second, tabs) = attach(&app, &rest, json!({"cdp_url": browser.cdp_url()})).await;
    let (third, _) = attach(&app, &rest, json!({"cdp_url": browser.cdp_url()})).await;
    assert_eq!(tabs.len(), 2);

    // instance.stop is not a way to end a Live Instance.
    let stop = call(
        &app,
        &rest,
        "instance.stop",
        json!({"instance_id": second.id}),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&stop, Error::InvalidTransition { from, .. } if from == "attached"),
        "{stop:?}"
    );

    // The daemon's shutdown detaches and leaves the browser.
    app.shutdown().await;
    assert_eq!(row(&app, second.id).await.state, InstanceState::Detached);
    assert_eq!(row(&app, third.id).await.state, InstanceState::Detached);
    assert!(browser.alive());
}

// A lost connection ends the Instance as lost.
#[tokio::test]
async fn lost_connection_ends_the_instance_as_lost() {
    let (dir, app) = scratch_app().await;
    let Some(mut browser) = Browser::start(dir.path()).await else {
        return;
    };
    let rest = client(&app, "rest").await;

    let (attached, _) = attach(&app, &rest, json!({"cdp_url": browser.cdp_url()})).await;
    browser.kill();

    let lost = time::timeout(
        LOST_WITHIN,
        wait_for("the instance to be lost", async || {
            row(&app, attached.id).await.state == InstanceState::Lost
        }),
    )
    .await;
    assert!(lost.is_ok(), "not lost within {LOST_WITHIN:?}");
}

// The user's own Tabs are not brr's load: with the Tab limits at the number of pages the user
// has, brr can still open a Tab in the Live browser and launch and use a managed one.
#[tokio::test]
async fn live_tabs_do_not_count_toward_limits() {
    let Some((dir, app, build_id)) = common::chrome_app(&[]).await else {
        return;
    };
    let Some(_browser) = Browser::start(dir.path()).await else {
        return;
    };
    let keep = common::KillOnDrop::new(&app, Vec::new());
    let rest = client(&app, "rest").await;
    app.set_settings(
        [
            ("limits.max_instances".to_owned(), json!(1)),
            ("limits.max_tabs_per_instance".to_owned(), json!(2)),
            ("limits.max_tabs_total".to_owned(), json!(2)),
        ]
        .into(),
    )
    .await
    .unwrap();

    let (live, tabs) = attach(&app, &rest, json!({"install_name": "chromium"})).await;
    assert_eq!(tabs.len(), 2);
    call(&app, &rest, "tab.open", json!({"instance_id": live.id}))
        .await
        .unwrap();

    common::add_profile(&app, build_id, "p", false, Default::default()).await;
    let launched = call(&app, &rest, "instance.launch", json!({"profile": "p"}))
        .await
        .unwrap();
    let managed: Instance = serde_json::from_value(launched["instance"].clone()).unwrap();
    keep.track(managed.id);
    call(&app, &rest, "tab.open", json!({"instance_id": managed.id}))
        .await
        .unwrap();

    call(
        &app,
        &rest,
        "instance.stop",
        json!({"instance_id": managed.id}),
    )
    .await
    .unwrap();
}

// A page brr opens at an internal URL is announced like any other; the open must not wait for
// an announcement the URL filter would swallow.
#[tokio::test]
async fn opening_internal_pages_answers() {
    let Some((dir, app, build_id)) = common::chrome_app(&[]).await else {
        return;
    };
    let Some(_browser) = Browser::start(dir.path()).await else {
        return;
    };
    let keep = common::KillOnDrop::new(&app, Vec::new());
    let rest = client(&app, "rest").await;
    let (live, _) = attach(&app, &rest, json!({"install_name": "chromium"})).await;
    common::add_profile(&app, build_id, "p", false, Default::default()).await;
    let launched = call(&app, &rest, "instance.launch", json!({"profile": "p"}))
        .await
        .unwrap();
    let managed: Instance = serde_json::from_value(launched["instance"].clone()).unwrap();
    keep.track(managed.id);

    for instance_id in [live.id, managed.id] {
        let opened = time::timeout(
            OPEN_ANSWERS_WITHIN,
            call(
                &app,
                &rest,
                "tab.open",
                json!({"instance_id": instance_id, "url": "chrome://version"}),
            ),
        )
        .await
        .expect("tab.open answers")
        .unwrap();
        let tab: Tab = serde_json::from_value(opened["tab"].clone()).unwrap();

        assert!(tab.opened_by_brr);
    }

    call(
        &app,
        &rest,
        "instance.stop",
        json!({"instance_id": managed.id}),
    )
    .await
    .unwrap();
}
