#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{Router, response::Html, routing::get};
use brr_core::{AdmissionMode, AssetKind, EndReason};
use brr_daemon::services::instances::LaunchRequest;
use brr_store::asset;
use tokio::{net::TcpListener, time};

use common::{KillOnDrop, add_profile, boot, chrome_app, listen};

const RULES: &str = "/ad.js$script\n";
const BLOCKED_PAGE: &str = r#"<html><body><script src="/ad.js"></script><p>page</p></body></html>"#;
// The markup of autoconsent's cookie-notice-checkboxes rule; its reject button removes the notice.
const COOKIE_PAGE: &str = "<div id=cookie-notice>\
    <button id=cookie-notice-accept class=cookie-notice__accept--custom>Accept</button>\
    <button id=cookie-notice-reject onclick=\"document.getElementById('cookie-notice').remove()\">Reject</button></div><p>content</p>";
const CLICK_DELAY_MS: &str = "3000";
const SETTLE: Duration = Duration::from_millis(500);

struct Fixture {
    base: String,
    list_hits: Arc<AtomicUsize>,
    ad_hits: Arc<AtomicUsize>,
}

async fn fixture() -> Fixture {
    let list_hits = Arc::new(AtomicUsize::new(0));
    let ad_hits = Arc::new(AtomicUsize::new(0));
    let (lists, ads) = (Arc::clone(&list_hits), Arc::clone(&ad_hits));
    let router = Router::new()
        .route(
            "/list.txt",
            get(move || async move {
                lists.fetch_add(1, Ordering::SeqCst);
                RULES
            }),
        )
        .route(
            "/ad.js",
            get(move || async move {
                ads.fetch_add(1, Ordering::SeqCst);
                ""
            }),
        )
        .route("/page", get(|| async { Html(BLOCKED_PAGE) }))
        .route("/cookies", get(|| async { Html(COOKIE_PAGE) }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await });

    Fixture {
        base,
        list_hits,
        ad_hits,
    }
}

fn list_env(fixture: &Fixture) -> String {
    format!("[\"{}/list.txt\"]", fixture.base)
}

#[tokio::test]
async fn profile_blocks_ads() {
    let fixture = fixture().await;
    let lists = list_env(&fixture);
    let Some((_dir, app, build_id)) =
        chrome_app(&[("BRR_CONTENT__ADBLOCK_LISTS", lists.as_str())]).await
    else {
        return;
    };
    let mut profile = add_profile(&app, build_id, "blocking", false, Default::default()).await;
    profile.settings.block_ads = true;
    let stored = profile.clone();
    app.store
        .run(move |c| brr_store::profile::update(c, &stored))
        .await
        .unwrap();

    let (running, tab) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile,
                display: None,
                priority: 0,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    app.instances
        .navigate(
            &running.id,
            &tab.id,
            format!("{}/page", fixture.base),
            brr_core::WaitUntil::Load,
            Duration::from_secs(20),
        )
        .await
        .unwrap();
    time::sleep(SETTLE).await;

    assert_eq!(fixture.ad_hits.load(Ordering::SeqCst), 0);
    let rows = app
        .store
        .run(|c| asset::by_kind(c, AssetKind::AdblockList))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0]
            .path
            .starts_with(app.paths.data_dir.join("assets/adblock"))
    );
    assert!(rows[0].path.exists());

    app.instances
        .stop(&app, &running.id, EndReason::Stop)
        .await
        .unwrap();
}

#[tokio::test]
async fn no_fetch_when_unused() {
    let fixture = fixture().await;
    let dir = tempfile::tempdir().unwrap();
    let lists = list_env(&fixture);
    let app = brr_daemon::App::start(boot(
        dir.path(),
        &[("BRR_CONTENT__ADBLOCK_LISTS", lists.as_str())],
    ))
    .await
    .unwrap();

    let options = app.content.options(&app, false, true).await;
    time::sleep(SETTLE).await;

    assert!(options.adblock.is_none());
    assert!(options.dismiss_cookies);
    assert_eq!(fixture.list_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn oneshot_dismiss_cookies() {
    let fixture = fixture().await;
    let Some((_dir, app, _build)) = chrome_app(&[]).await else {
        return;
    };
    let _browsers = KillOnDrop::new(&app, Vec::new());
    let addr = listen(&app).await;
    let url = format!("{}/cookies", fixture.base);

    let plain = content(addr, &[("url", &url)]).await;
    assert!(plain.contains("cookie-notice"));

    let dismissed = content(
        addr,
        &[
            ("url", &url),
            ("dismiss_cookies", "true"),
            ("delay", CLICK_DELAY_MS),
        ],
    )
    .await;
    assert!(!dismissed.contains("cookie-notice"));
}

async fn content(addr: std::net::SocketAddr, query: &[(&str, &str)]) -> String {
    let response = reqwest::Client::new()
        .get(format!("http://{addr}/v1/content"))
        .query(query)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    response.text().await.unwrap()
}
