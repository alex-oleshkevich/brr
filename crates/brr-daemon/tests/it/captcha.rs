#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{Json, Router, extract::State, http::Uri, response::Html, routing::get};
use brr_core::{ClientKind, Error, IdleOverrides, Instance, InstanceId, Order, Page, Tab, TabId};
use brr_daemon::{App, CallClient, ops};
use brr_store::event;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional},
    net::TcpStream,
    sync::Notify,
    time,
};

use common::{KillOnDrop, add_profile, chrome_app, ctx, events_of, listen, scratch_app};

const PROFILE: &str = "p";
const TRACE: &str = "ip=203.0.113.9\nloc=NL\n";
// Below the reserved price of one solve: the first passes on an empty day, a second behind it does not.
const TIGHT_USD_MICROS: u64 = 4_000;
const PASSWORD: &str = "S3cret-pw-9";
const API_KEY: &str = "2cap-KEY-1234";
const WAIT: Duration = Duration::from_secs(40);
const POLL: Duration = Duration::from_millis(100);
// The widget's callback is where the real reCAPTCHA script would call back once solved.
const CAPTCHA_PAGE: &str = r#"<div class="g-recaptcha" data-sitekey="K2"></div>
<textarea name="g-recaptcha-response"></textarea>
<script>window.___grecaptcha_cfg = {clients: {0: {w: {callback: (t) => { document.title = 'solved:' + t; }}}}};</script>"#;

#[derive(Default)]
struct Fake {
    tasks: Mutex<Vec<Value>>,
    reject_results: AtomicBool,
    // While set, a task result is not ready until `release` is notified.
    hold_results: AtomicBool,
    release: Notify,
}

async fn handle(State(fake): State<Arc<Fake>>, uri: Uri, Json(body): Json<Value>) -> Json<Value> {
    Json(match uri.path() {
        "/createTask" => {
            fake.tasks.lock().unwrap().push(body["task"].clone());
            json!({"errorId": 0, "taskId": 7})
        }
        "/getTaskResult" if fake.hold_results.load(Ordering::SeqCst) => {
            fake.release.notified().await;
            json!({
                "errorId": 0,
                "status": "ready",
                "cost": "0.00299",
                "solution": {"gRecaptchaResponse": "tok"},
            })
        }
        "/getTaskResult" if fake.reject_results.load(Ordering::SeqCst) => {
            json!({"errorId": 1, "errorCode": "ERROR_CAPTCHA_UNSOLVABLE"})
        }
        "/getTaskResult" => json!({
            "errorId": 0,
            "status": "ready",
            "cost": "0.00299",
            "solution": {"gRecaptchaResponse": "tok"},
        }),
        "/getBalance" => json!({"errorId": 0, "balance": 1.5}),
        _ => json!({"errorId": 1, "errorCode": "ERROR_UNEXPECTED"}),
    })
}

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    origin
}

struct Fixture {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    browsers: KillOnDrop,
    client: CallClient,
    fake: Arc<Fake>,
    page_url: String,
}

async fn fixture() -> Option<Fixture> {
    let fake = Arc::new(Fake::default());
    let provider_url = serve(Router::new().fallback(handle).with_state(fake.clone())).await;
    let page_url = serve(
        Router::new()
            .route("/", get(async || Html(CAPTCHA_PAGE)))
            .route("/trace", get(async || TRACE)),
    )
    .await;

    let check_url = format!("{page_url}/trace");
    let (dir, app, build_id) = chrome_app(&[
        ("BRR_CAPTCHA__PROVIDER_URL", provider_url.as_str()),
        ("BRR_PROXY__CHECK_URL", check_url.as_str()),
    ])
    .await?;
    add_profile(&app, build_id, PROFILE, false, IdleOverrides::default()).await;
    let created = app
        .clients
        .create(&app, ClientKind::Rest, "captcha".to_string())
        .await
        .unwrap();
    let client = app.clients.resolve(&app, &created.id).await.unwrap();
    let browsers = KillOnDrop::new(&app, Vec::new());

    Some(Fixture {
        _dir: dir,
        app,
        browsers,
        client,
        fake,
        page_url,
    })
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

impl Fixture {
    async fn call(&self, name: &str, input: Value) -> Result<Value, Error> {
        call(&self.app, &self.client, name, input).await
    }

    async fn set(&self, input: Value) {
        self.call("captcha.settings_set", input).await.unwrap();
    }

    async fn launch(&self) -> (InstanceId, TabId) {
        self.launch_profile(PROFILE).await
    }

    async fn launch_profile(&self, profile: &str) -> (InstanceId, TabId) {
        let launched = self
            .call(
                "instance.launch",
                json!({"profile": profile, "url": format!("{}/", self.page_url)}),
            )
            .await
            .unwrap();
        let instance: Instance = serde_json::from_value(launched["instance"].clone()).unwrap();
        self.browsers.track(instance.id);
        let tab: Tab = serde_json::from_value(launched["tab"].clone()).unwrap();

        (instance.id, tab.id)
    }

    async fn title(&self, instance_id: InstanceId, tab_id: TabId) -> String {
        let page = self
            .app
            .instances
            .page(&instance_id, &tab_id)
            .await
            .unwrap();

        page.evaluate("document.title", Duration::from_secs(5))
            .await
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn events(&self, kind: &str, instance_id: InstanceId) -> Vec<Value> {
        events_of(&self.app, kind, instance_id).await
    }

    // Not tied to an Instance: the event belongs to the daemon.
    async fn budget_events(&self) -> Vec<Value> {
        let query = event::Query {
            since: None,
            kinds: vec!["captcha.budget_exhausted".to_string()],
            subject: None,
            instance_id: None,
            from_ts: None,
            to_ts: None,
            order: Order::Asc,
            page: Page::new(Some(100), None),
        };
        let (events, _) = self
            .app
            .store
            .run(move |c| event::list(c, &query))
            .await
            .unwrap();

        events.into_iter().map(|event| event.data).collect()
    }

    async fn wait_events(&self, kind: &str, instance_id: InstanceId, count: usize) -> Vec<Value> {
        time::timeout(WAIT, async {
            loop {
                let events = self.events(kind, instance_id).await;
                if events.len() >= count {
                    return events;
                }
                time::sleep(POLL).await;
            }
        })
        .await
        .unwrap()
    }

    async fn open_tab(&self, instance_id: InstanceId) -> TabId {
        let opened = self
            .call(
                "tab.open",
                json!({"instance_id": instance_id, "url": format!("{}/", self.page_url)}),
            )
            .await
            .unwrap();
        let tab: Tab = serde_json::from_value(opened["tab"].clone()).unwrap();

        tab.id
    }

    async fn wait_tasks(&self, count: usize) {
        time::timeout(WAIT, async {
            while self.fake.tasks.lock().unwrap().len() < count {
                time::sleep(POLL).await;
            }
        })
        .await
        .unwrap();
    }

    async fn solves(&self) -> Vec<Value> {
        let solves = self.call("captcha.solves", json!({})).await.unwrap();

        solves["solves"].as_array().unwrap().clone()
    }
}

#[tokio::test]
async fn auto_solves_on_load() {
    let Some(f) = fixture().await else {
        return;
    };
    f.set(json!({"api_key": API_KEY})).await;
    let (instance_id, tab_id) = f.launch().await;

    let solved = f.wait_events("captcha.solved", instance_id, 1).await;
    assert_eq!(solved[0]["cost_usd"], 0.00299);
    assert_eq!(solved[0]["type"], "recaptcha_v2");
    let detected = f.events("captcha.detected", instance_id).await;
    assert_eq!(detected.len(), 1);
    assert_eq!(detected[0]["site"], f.page_url);

    assert_eq!(f.title(instance_id, tab_id).await, "solved:tok");
    let solves = f.solves().await;
    assert_eq!(solves.len(), 1);
    assert_eq!(solves[0]["result"], "solved");
    assert_eq!(solves[0]["cost_usd_micros"], 2990);
    assert_eq!(solves[0]["site"], f.page_url);
    assert_eq!(
        f.fake.tasks.lock().unwrap()[0]["type"],
        "RecaptchaV2TaskProxyless"
    );
}

#[tokio::test]
async fn budget_then_human() {
    let Some(f) = fixture().await else {
        return;
    };
    f.set(json!({
        "api_key": API_KEY,
        "default_policy": "on_demand",
        "daily_solves": 1,
        "human_wait_s": 5,
    }))
    .await;
    let (instance_id, tab_id) = f.launch().await;
    let input = json!({"tab_id": tab_id});

    let first = f.call("tab.solve_captcha", input.clone()).await.unwrap();
    assert_eq!(first["solved_by"], "provider");
    assert_eq!(first["detected"]["type"], "recaptcha_v2");
    assert_eq!(f.title(instance_id, tab_id).await, "solved:tok");

    let started = Instant::now();
    let second = f.call("tab.solve_captcha", input.clone()).await;
    assert_eq!(second, Err(Error::CaptchaFailed));
    assert!(started.elapsed() >= Duration::from_secs(4));

    let waiting = f.events("tab.awaiting_human", instance_id).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0]["reason"], "captcha");
    assert_eq!(f.budget_events().await.len(), 1);

    let strict = f
        .call(
            "tab.solve_captcha",
            json!({"tab_id": tab_id, "human_fallback": false}),
        )
        .await;
    assert_eq!(strict, Err(Error::CaptchaBudgetExceeded));
    assert_eq!(f.budget_events().await.len(), 1);
}

#[tokio::test]
async fn off_without_key() {
    let Some(f) = fixture().await else {
        return;
    };
    let (instance_id, tab_id) = f.launch().await;

    // The page is loaded; give the detector a bounded window in which it must stay silent.
    let page = f.app.instances.page(&instance_id, &tab_id).await.unwrap();
    time::timeout(WAIT, async {
        while page
            .evaluate(
                "document.querySelector('.g-recaptcha') !== null",
                Duration::from_secs(5),
            )
            .await
            .ok()
            != Some(json!(true))
        {
            time::sleep(POLL).await;
        }
    })
    .await
    .unwrap();
    time::sleep(Duration::from_secs(2)).await;
    for kind in [
        "captcha.detected",
        "captcha.solved",
        "captcha.failed",
        "captcha.budget_exhausted",
    ] {
        assert!(f.events(kind, instance_id).await.is_empty(), "{kind}");
    }
    assert!(f.budget_events().await.is_empty());
    assert_eq!(f.title(instance_id, tab_id).await, "");

    let balance = f.call("captcha.balance", json!({})).await;
    assert!(matches!(balance, Err(Error::InvalidArgument { field, .. }) if field == "api_key"));
}

#[tokio::test]
async fn settings_round_trip() {
    let (dir, app) = scratch_app().await;
    let client = ctx(&app).client;
    let mut outputs = Vec::new();

    let set = call(
        &app,
        &client,
        "captcha.settings_set",
        json!({"api_key": API_KEY, "daily_usd_micros": 5_100_000}),
    )
    .await
    .unwrap();
    outputs.push(set);
    let got = call(&app, &client, "captcha.settings_get", json!({}))
        .await
        .unwrap();
    assert_eq!(got["api_key"], json!({"set": true, "hint": "…1234"}));
    assert_eq!(got["daily_usd_micros"], 5_100_000);
    assert_eq!(got["provider"], "2captcha");
    assert_eq!(got["spent_today_usd_micros"], 0);
    outputs.push(got);

    let config = fs::read_to_string(common::paths(dir.path()).config_file()).unwrap();
    assert!(config.contains("daily_usd = 5.1"), "{config}");

    let kept = call(
        &app,
        &client,
        "captcha.settings_set",
        json!({"default_policy": "human"}),
    )
    .await
    .unwrap();
    assert_eq!(kept["default_policy"], "human");
    assert_eq!(kept["api_key"]["set"], true);
    outputs.push(kept);

    let removed = call(
        &app,
        &client,
        "captcha.settings_set",
        json!({"api_key": null}),
    )
    .await
    .unwrap();
    assert_eq!(removed["api_key"], Value::Null);
    outputs.push(removed);

    for output in outputs {
        assert!(!output.to_string().contains(API_KEY));
    }
    assert!(!config.contains(API_KEY));
}

#[tokio::test]
async fn metrics() {
    let Some(f) = fixture().await else {
        return;
    };
    f.set(json!({"api_key": API_KEY, "default_policy": "on_demand"}))
        .await;
    let (_, tab_id) = f.launch().await;
    let input = json!({"tab_id": tab_id, "human_fallback": false});

    f.call("tab.solve_captcha", input.clone()).await.unwrap();
    f.fake.reject_results.store(true, Ordering::SeqCst);
    let failed = f.call("tab.solve_captcha", input).await;
    assert_eq!(failed, Err(Error::CaptchaFailed));

    let addr = listen(&f.app).await;
    let body = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        body.contains("brr_captcha_solves_total{result=\"solved\"} 1\n"),
        "{body}"
    );
    assert!(
        body.contains("brr_captcha_solves_total{result=\"failed\"} 1\n"),
        "{body}"
    );
    assert!(
        body.contains("brr_captcha_spend_usd_total 0.002990\n"),
        "{body}"
    );
    let rows = f.solves().await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["result"], "failed");
}

// A plain forwarding HTTP proxy: Chrome sends the page load through it, so it has to work.
async fn forwarding_proxy() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (client, _) = listener.accept().await.unwrap();
            tokio::spawn(forward(client));
        }
    });

    port
}

async fn forward(mut client: TcpStream) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        match client.read_u8().await {
            Ok(byte) => head.push(byte),
            Err(_) => return,
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let mut parts = head.lines().next().unwrap_or_default().split(' ');
    let (method, target) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    let url = target.strip_prefix("http://");
    let authority = url.map_or(target, |url| url.split('/').next().unwrap_or_default());
    let Ok(mut origin) = TcpStream::connect(authority).await else {
        return;
    };

    if method == "CONNECT" {
        client.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
    } else {
        let path = &url.unwrap_or_default()[authority.len()..];
        let request =
            format!("{method} {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
        origin.write_all(request.as_bytes()).await.unwrap();
    }
    copy_bidirectional(&mut client, &mut origin).await.ok();
}

#[tokio::test]
async fn proxy_passed_to_provider() {
    let Some(f) = fixture().await else {
        return;
    };
    let proxy_port = forwarding_proxy().await;
    let created = f
        .call(
            "proxy.create",
            json!({"scheme": "http", "host": "127.0.0.1", "port": proxy_port}),
        )
        .await
        .unwrap();
    f.call(
        "profile.create",
        json!({"name": "proxied", "template": "chromium-headless"}),
    )
    .await
    .unwrap();
    let profile = f
        .call("profile.get", json!({"profile": "proxied"}))
        .await
        .unwrap();
    let mut settings = profile["settings"].clone();
    settings["proxy"] = json!({"proxy_id": created["id"]});
    f.call(
        "profile.update",
        json!({"profile": "proxied", "settings": settings}),
    )
    .await
    .unwrap();
    f.set(json!({"api_key": API_KEY})).await;

    let (instance_id, _) = f.launch_profile("proxied").await;

    f.wait_events("captcha.solved", instance_id, 1).await;
    let task = f.fake.tasks.lock().unwrap()[0].clone();
    // The Profile's upstream, not the forwarder brr starts for the browser.
    assert_eq!(task["type"], "RecaptchaV2Task");
    assert_eq!(task["proxyAddress"], "127.0.0.1");
    assert_eq!(task["proxyPort"], proxy_port);
}

// A login holds the evaluate lock and has typed a secret; the provider's answer must still land.
#[tokio::test]
async fn applies_on_a_page_holding_a_secret() {
    let Some(f) = fixture().await else {
        return;
    };
    f.set(json!({"api_key": API_KEY, "default_policy": "on_demand"}))
        .await;
    let (instance_id, tab_id) = f.launch().await;
    let page = f.app.instances.page(&instance_id, &tab_id).await.unwrap();
    let _login = page.lock_evaluate();
    page.state().add_secret(PASSWORD);

    let solved = f
        .call("tab.solve_captcha", json!({"tab_id": tab_id}))
        .await
        .unwrap();

    assert_eq!(solved["solved_by"], "provider");
    let title = page.evaluate_utility("document.title").await.unwrap();
    assert_eq!(title, "solved:tok");
    assert_eq!(f.solves().await.len(), 1);
    assert!(f.events("captcha.failed", instance_id).await.is_empty());
}

// Solves that are still with the provider count against the daily USD cap at a reserved price.
#[tokio::test]
async fn in_flight_solves_reserve_usd() {
    let Some(f) = fixture().await else {
        return;
    };
    f.set(json!({
        "api_key": API_KEY,
        "default_policy": "on_demand",
        "daily_usd_micros": TIGHT_USD_MICROS,
    }))
    .await;
    f.fake.hold_results.store(true, Ordering::SeqCst);
    let (instance_id, first_tab) = f.launch().await;
    let second_tab = f.open_tab(instance_id).await;
    let third_tab = f.open_tab(instance_id).await;
    let strict = |tab_id: TabId| json!({"tab_id": tab_id, "human_fallback": false});

    let (app, client) = (f.app.clone(), f.client.clone());
    let first =
        tokio::spawn(
            async move { call(&app, &client, "tab.solve_captcha", strict(first_tab)).await },
        );
    f.wait_tasks(1).await;

    for tab_id in [second_tab, third_tab] {
        let refused = f.call("tab.solve_captcha", strict(tab_id)).await;
        assert_eq!(refused, Err(Error::CaptchaBudgetExceeded));
    }
    assert_eq!(f.fake.tasks.lock().unwrap().len(), 1);

    f.fake.release.notify_one();
    let first = first.await.unwrap().unwrap();
    assert_eq!(first["solved_by"], "provider");

    // The recorded cost replaced the reservation: room for another one.
    f.fake.hold_results.store(false, Ordering::SeqCst);
    let again = f
        .call("tab.solve_captcha", strict(second_tab))
        .await
        .unwrap();
    assert_eq!(again["solved_by"], "provider");
    assert_eq!(f.solves().await.len(), 2);
}
