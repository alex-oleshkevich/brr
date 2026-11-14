#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{collections::HashMap, sync::Arc, time::Duration};

use axum::{
    Form, Router,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use brr_core::{
    ConflictReason, Error, IdleOverrides, Instance, InstanceId, LoginStatus, Page as Paging, Tab,
    TabId,
};
use brr_daemon::{App, CallClient, ops};
use brr_store::event;
use serde_json::{Value, json};
use tokio::sync::Notify;

use common::{KillOnDrop, add_profile, chrome_app, ctx, scratch_app};

const PROFILE: &str = "p";
const USERNAME: &str = "u1";
const PASSWORD: &str = "S3cret-pw-9";
const START_PAGE: &str = "data:text/html,<p>start</p>";
const COOKIE: &str = "session=ok";
const EXPECT_TEXT: &str = "Logged in as";
const CHECK_RESULT: &str = "login.check_result";
const RESULT_TIMEOUT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);

const LOGIN_FORM: &str = "<form method=post action=/signin><input id=u name=username><input id=p type=password name=password><button type=submit>Sign in</button></form>";
// What a site shows right after signing in: a change-password form and the account banner.
// The element appears after the load event, as on a page that renders client-side.
const LATE_PAGE: &str = "<script>addEventListener('load', () => setTimeout(() => { document.body.innerHTML = '<h1 id=late>Hi</h1>'; }, 1500))</script>";
const CHANGE_PASSWORD: &str = "<p>Logged in as u1</p><form><input type=password name=new></form>";

async fn serve(slow: Arc<Notify>) -> String {
    let app = Router::new()
        .route(
            "/slow",
            get(async move || {
                slow.notified().await;
                Html("<p>late</p>")
            }),
        )
        .route("/login", get(async || Html(LOGIN_FORM)))
        .route(
            "/signin",
            post(
                async |Form(form): Form<HashMap<String, String>>| -> Response {
                    let accepted = form.get("username").map(String::as_str) == Some(USERNAME)
                        && form.get("password").map(String::as_str) == Some(PASSWORD);
                    if !accepted {
                        return Html(LOGIN_FORM).into_response();
                    }

                    let mut headers = HeaderMap::new();
                    headers.insert(
                        header::SET_COOKIE,
                        HeaderValue::from_str(&format!("{COOKIE}; Path=/")).unwrap(),
                    );

                    (headers, Redirect::to("/change")).into_response()
                },
            ),
        )
        .route("/change", get(async || Html(CHANGE_PASSWORD)))
        .route("/late", get(async || Html(LATE_PAGE)))
        .route(
            "/account",
            get(async |headers: HeaderMap| -> Response {
                let signed_in = headers
                    .get(header::COOKIE)
                    .and_then(|cookie| cookie.to_str().ok())
                    .is_some_and(|cookie| cookie.contains(COOKIE));
                if signed_in {
                    Html("<p>Logged in as u1</p>").into_response()
                } else {
                    (StatusCode::SEE_OTHER, [(header::LOCATION, "/login")]).into_response()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    origin
}

struct Fixture {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    browsers: KillOnDrop,
    client: CallClient,
    origin: String,
    // The /slow page answers once this is notified.
    slow: Arc<Notify>,
}

async fn call(f: &Fixture, name: &str, input: Value) -> Result<Value, Error> {
    let mut context = ctx(&f.app);
    context.client = f.client.clone();

    ops::dispatch(context, ops::find(name).unwrap(), input).await
}

async fn fixture() -> Option<Fixture> {
    let (dir, app, build_id) = chrome_app(&[]).await?;
    add_profile(&app, build_id, PROFILE, false, IdleOverrides::default()).await;
    let client = ctx(&app).client;
    let browsers = KillOnDrop::new(&app, Vec::new());

    let slow = Arc::new(Notify::new());

    Some(Fixture {
        _dir: dir,
        app,
        browsers,
        client,
        origin: serve(slow.clone()).await,
        slow,
    })
}

async fn launch(f: &Fixture) -> (InstanceId, Tab) {
    let launched = call(
        f,
        "instance.launch",
        json!({"profile": PROFILE, "url": START_PAGE}),
    )
    .await
    .unwrap();
    let instance: Instance = serde_json::from_value(launched["instance"].clone()).unwrap();
    f.browsers.track(instance.id);

    (
        instance.id,
        serde_json::from_value(launched["tab"].clone()).unwrap(),
    )
}

async fn create_check(f: &Fixture, extra: Value) -> Value {
    let mut input = json!({
        "profile": PROFILE,
        "site": f.origin,
        "url": format!("{}/account", f.origin),
        "expect_text": EXPECT_TEXT,
    });
    input
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());

    call(f, "login_check.create", input).await.unwrap()
}

async fn credential(f: &Fixture) -> Value {
    call(
        f,
        "credential.create",
        json!({
            "site": f.origin,
            "login_url": format!("{}/login", f.origin),
            "username": USERNAME,
            "password": PASSWORD,
            "profile_ids": [PROFILE],
        }),
    )
    .await
    .unwrap()
}

async fn results(app: &Arc<App>) -> Vec<Value> {
    events_of(app, CHECK_RESULT).await
}

async fn events_of(app: &Arc<App>, kind: &str) -> Vec<Value> {
    let query = event::Query {
        since: None,
        kinds: vec![kind.to_owned()],
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: brr_core::Order::Asc,
        page: Paging::new(Some(1000), None),
    };
    let (events, _) = app
        .store
        .run(move |c| event::list(c, &query))
        .await
        .unwrap();

    events.into_iter().map(|event| event.data).collect()
}

async fn open_tabs(f: &Fixture, instance_id: InstanceId) -> Vec<TabId> {
    f.app
        .instances
        .view(&instance_id)
        .unwrap()
        .tabs
        .iter()
        .filter(|tab| tab.state != brr_core::TabState::Closed)
        .map(|tab| tab.id)
        .collect()
}

#[tokio::test]
async fn runs_on_running() {
    let Some(f) = fixture().await else { return };
    create_check(&f, json!({})).await;

    launch(&f).await;

    let found = tokio::time::timeout(RESULT_TIMEOUT, async {
        loop {
            let found = results(&f.app).await;
            if !found.is_empty() {
                return found;
            }
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(found[0]["status"], "failed");
    let listed = call(&f, "profile.get", json!({"profile": PROFILE}))
        .await
        .unwrap();
    assert_eq!(listed["login_status"], "failed");
}

#[tokio::test]
async fn auto_relogin() {
    let Some(f) = fixture().await else { return };
    let credential = credential(&f).await;
    let check = create_check(
        &f,
        json!({"credential_id": credential["id"], "auto_relogin": true}),
    )
    .await;
    let (instance_id, _) = launch(&f).await;
    let tabs_before = open_tabs(&f, instance_id).await;

    let ran = call(
        &f,
        "login_check.run",
        json!({"login_check_id": check["id"]}),
    )
    .await
    .unwrap();

    assert_eq!(ran["last_status"], "ok");
    // The browser may close its own blank tab meanwhile; the check's Tab must be gone.
    assert!(
        open_tabs(&f, instance_id)
            .await
            .iter()
            .all(|tab| tabs_before.contains(tab))
    );
}

// The caller of login_check.run gives up while the check's page loads: the check still finishes,
// records its result and closes its Tab.
#[tokio::test]
async fn dropped_run_still_closes_its_tab() {
    let Some(f) = fixture().await else { return };
    let check = create_check(&f, json!({"url": format!("{}/slow", f.origin)})).await;
    let (instance_id, _) = launch(&f).await;
    // The check at launch is the first run of it; wait it out so only the manual run is left.
    f.slow.notify_one();
    tokio::time::timeout(RESULT_TIMEOUT, async {
        while results(&f.app).await.is_empty() {
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .unwrap();
    let recorded = results(&f.app).await.len();
    let tabs_before = open_tabs(&f, instance_id).await;

    let (app, client, check_id) = (f.app.clone(), f.client.clone(), check["id"].clone());
    let run = tokio::spawn(async move {
        let mut context = ctx(&app);
        context.client = client;

        ops::dispatch(
            context,
            ops::find("login_check.run").unwrap(),
            json!({"login_check_id": check_id}),
        )
        .await
    });
    tokio::time::timeout(RESULT_TIMEOUT, async {
        while open_tabs(&f, instance_id).await.len() <= tabs_before.len() {
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .unwrap();
    run.abort();
    f.slow.notify_one();

    tokio::time::timeout(RESULT_TIMEOUT, async {
        while results(&f.app).await.len() == recorded
            || open_tabs(&f, instance_id).await.len() > tabs_before.len()
        {
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn validation() {
    let (_dir, app) = scratch_app().await;
    let call = async |name: &str, input: Value| {
        ops::dispatch(ctx(&app), ops::find(name).unwrap(), input).await
    };
    call(
        "profile.create",
        json!({"name": PROFILE, "template": "chromium-headless"}),
    )
    .await
    .unwrap();
    let both = call(
        "login_check.create",
        json!({
            "profile": PROFILE,
            "site": "https://a.test",
            "url": "https://a.test/account",
            "expect_text": "x",
            "expect_selector": "#x",
        }),
    )
    .await;
    let created = call(
        "login_check.create",
        json!({"profile": PROFILE, "site": "https://a.test", "url": "https://a.test/account", "expect_text": "x"}),
    )
    .await
    .unwrap();
    let not_running = call("login_check.run", json!({"login_check_id": created["id"]})).await;

    assert!(
        matches!(both, Err(Error::InvalidArgument { field, .. }) if field == "expect_selector")
    );
    assert!(matches!(
        not_running,
        Err(Error::Conflict {
            reason: ConflictReason::ProfileNotRunning,
            ..
        })
    ));
    assert_eq!(
        created["last_status"],
        serde_json::to_value(LoginStatus::Unknown).unwrap()
    );
}

#[tokio::test]
async fn login_uses_check() {
    let Some(f) = fixture().await else { return };
    let credential = credential(&f).await;
    create_check(&f, json!({})).await;
    let (_, tab) = launch(&f).await;

    let done = call(
        &f,
        "tab.login",
        json!({"tab_id": tab.id, "credential_id": credential["id"]}),
    )
    .await
    .unwrap();

    // The page after signing in still shows a password field: the check decided.
    assert_eq!(done["ok"], true);
    let listed = call(&f, "login_check.list", json!({"profile": PROFILE}))
        .await
        .unwrap();
    assert_eq!(listed["login_checks"][0]["last_status"], "ok");
}

#[tokio::test]
async fn selector_check_waits_for_a_late_element() {
    let Some(f) = fixture().await else { return };
    launch(&f).await;
    let check = |selector: &str| {
        json!({
            "profile": PROFILE,
            "site": f.origin,
            "url": format!("{}/late", f.origin),
            "expect_selector": selector,
        })
    };
    let late = call(&f, "login_check.create", check("h1#late"))
        .await
        .unwrap();
    let missing = call(&f, "login_check.create", check("h1#never"))
        .await
        .unwrap();

    let found = call(&f, "login_check.run", json!({"login_check_id": late["id"]}))
        .await
        .unwrap();
    let absent = call(
        &f,
        "login_check.run",
        json!({"login_check_id": missing["id"]}),
    )
    .await
    .unwrap();

    assert_eq!(found["last_status"], "ok");
    assert_eq!(absent["last_status"], "failed");
}

#[tokio::test]
async fn relogin_suspends_after_three_failures() {
    let Some(f) = fixture().await else { return };
    let wrong = call(
        &f,
        "credential.create",
        json!({
            "site": f.origin,
            "login_url": format!("{}/login", f.origin),
            "username": USERNAME,
            "password": "wrong",
            "profile_ids": [PROFILE],
        }),
    )
    .await
    .unwrap();
    let check = create_check(
        &f,
        json!({"credential_id": wrong["id"], "auto_relogin": true}),
    )
    .await;
    launch(&f).await;
    let run = async || {
        call(
            &f,
            "login_check.run",
            json!({"login_check_id": check["id"]}),
        )
        .await
        .unwrap()
    };

    for _ in 0..3 {
        assert_eq!(run().await["last_status"], "failed");
    }
    let suspended = events_of(&f.app, "login.relogin_suspended").await;
    assert_eq!(suspended.len(), 1, "{suspended:?}");
    assert_eq!(suspended[0]["failures"], 3);
    let attempts = events_of(&f.app, "login.performed").await.len();

    assert_eq!(run().await["last_status"], "failed");

    assert_eq!(events_of(&f.app, "login.performed").await.len(), attempts);
    assert_eq!(events_of(&f.app, "login.relogin_suspended").await.len(), 1);
}
