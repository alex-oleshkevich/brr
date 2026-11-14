#![allow(clippy::unwrap_used)]

use crate::common;

use std::sync::Arc;

use brr_core::{AdmissionMode, IdleOverrides};
use brr_daemon::{App, ops, services::instances::LaunchRequest};
use serde_json::json;

use common::{KillOnDrop, add_profile, chrome_app, ctx, listen, scratch_app};

const MIN_BROWSER_RSS_BYTES: u64 = 20 * 1024 * 1024;
const CONTENT_TYPE_PREFIX: &str = "text/plain; version=0.0.4";
const DOCTOR_CHECKS: [&str; 12] = [
    "data_dir",
    "config_dir",
    "listen",
    "keyring",
    "xvfb",
    "display",
    "dev_shm",
    "user_namespaces",
    "chromium_build",
    "disk_floor",
    "docker",
    "xz",
];

async fn scrape(app: &Arc<App>) -> (reqwest::header::HeaderMap, String) {
    let addr = listen(app).await;
    let response = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let headers = response.headers().clone();

    (headers, response.text().await.unwrap())
}

#[tokio::test]
async fn metrics_format() {
    let (_dir, app) = scratch_app().await;

    let (headers, body) = scrape(&app).await;

    let content_type = headers["content-type"].to_str().unwrap();
    assert!(
        content_type.starts_with(CONTENT_TYPE_PREFIX),
        "{content_type}"
    );
    assert!(body.contains("brr_admission_queue_depth 0\n"), "{body}");
    for line in body.lines() {
        if line.starts_with('#') {
            continue;
        }
        let (series, value) = line.rsplit_once(' ').unwrap();
        assert!(series.starts_with("brr_"), "{line}");
        assert!(value.parse::<f64>().is_ok(), "{line}");
    }
}

#[tokio::test]
async fn metrics_running() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, "scraped", false, IdleOverrides::default()).await;
    let (running, _) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile,
                display: None,
                priority: 5,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    let (_, body) = scrape(&app).await;

    assert!(
        body.contains("brr_instances{state=\"running\",engine=\"chromium\"} 1\n"),
        "{body}"
    );
    assert!(body.contains("brr_tabs{state=\"ready\"} 1\n"), "{body}");
    let rss_series = format!("brr_instance_rss_bytes{{instance=\"{}\"}} ", running.id);
    let rss: u64 = body
        .lines()
        .find_map(|line| line.strip_prefix(&rss_series))
        .unwrap()
        .parse()
        .unwrap();
    assert!(rss > MIN_BROWSER_RSS_BYTES);
}

#[tokio::test]
async fn doctor_op() {
    let (_dir, app) = scratch_app().await;

    let out = ops::dispatch(ctx(&app), ops::find("system.doctor").unwrap(), json!({}))
        .await
        .unwrap();

    let checks = out["checks"].as_array().unwrap();
    let names: Vec<&str> = checks
        .iter()
        .map(|check| check["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, DOCTOR_CHECKS);

    let chromium = &checks[8];
    assert_eq!(chromium["status"], "fail");
    assert!(
        chromium["detail"]
            .as_str()
            .unwrap()
            .contains("brr build detect")
    );
}
