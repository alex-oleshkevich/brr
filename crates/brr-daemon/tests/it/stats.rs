#![allow(clippy::unwrap_used)]

use crate::common;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use axum::{Json, Router, extract::State, routing::post};
use brr_core::{
    Build, BuildId, BuildSource, BuildState, CaptchaResult, CaptchaSolve, CaptchaSolveId,
    CaptchaType, Engine, Error, InstanceKind, InstanceState, ProxyId, Traffic,
    clock::local_day_start, now_ms,
};
use brr_daemon::{App, ops};
use brr_store::{build, captcha_solve, instance};
use serde_json::{Value, json};

use common::{boot, ctx, instance_row, scratch_app};

const HOUR_MS: i64 = 3_600_000;
const SOLVE_COST: u64 = 2_990;
const OLD_COST: u64 = 1_000;
const API_KEY: &str = "2cap-KEY-1234";
const OTHER_KEY: &str = "2cap-KEY-5678";

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    ops::dispatch(ctx(app), ops::find(name).unwrap(), input).await
}

fn solve(ts: i64, cost_usd_micros: u64, result: CaptchaResult) -> CaptchaSolve {
    CaptchaSolve {
        id: CaptchaSolveId::new(),
        ts,
        profile_id: None,
        tab_id: None,
        site: "https://example.com".to_string(),
        captcha_type: CaptchaType::RecaptchaV2,
        provider: "2captcha".to_string(),
        cost_usd_micros,
        duration_ms: 1_000,
        result,
        error: None,
    }
}

fn day_start(days_ago: usize) -> i64 {
    (0..days_ago).fold(local_day_start(now_ms()), |start, _| {
        local_day_start(start - 1)
    })
}

fn invalid_field(result: Result<Value, Error>) -> String {
    match result {
        Err(Error::InvalidArgument { field, .. }) => field,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[tokio::test]
async fn captcha_stats_group_by_local_day_and_fill_empty_days() {
    let (_dir, app) = scratch_app().await;
    let rows = [
        solve(day_start(0), SOLVE_COST, CaptchaResult::Solved),
        solve(day_start(0), 0, CaptchaResult::Failed),
        solve(day_start(2), OLD_COST, CaptchaResult::Solved),
        solve(day_start(10), OLD_COST, CaptchaResult::Solved),
    ];
    app.store
        .run(move |c| {
            rows.iter()
                .try_for_each(|row| captcha_solve::insert(c, row))
        })
        .await
        .unwrap();

    let stats = call(&app, "captcha.stats", json!({})).await.unwrap();
    let days = stats["days"].as_array().unwrap();

    assert_eq!(days.len(), 7);
    for (index, day) in days.iter().enumerate() {
        assert_eq!(day["start_ms"], day_start(6 - index));
    }
    assert_eq!(days[6]["spend_usd_micros"], SOLVE_COST);
    assert_eq!(days[6]["solves"], 1);
    assert_eq!(days[6]["failures"], 1);
    assert_eq!(days[4]["spend_usd_micros"], OLD_COST);
    assert_eq!(days[4]["solves"], 1);
    assert_eq!(days[4]["failures"], 0);
    for index in [0, 1, 2, 3, 5] {
        assert_eq!(
            days[index],
            json!({
                "day": days[index]["day"],
                "start_ms": day_start(6 - index),
                "spend_usd_micros": 0,
                "solves": 0,
                "failures": 0,
            })
        );
    }
    assert_ne!(days[5]["day"], days[6]["day"]);

    let month = call(&app, "captcha.stats", json!({"days": 30}))
        .await
        .unwrap();
    let month = month["days"].as_array().unwrap();
    assert_eq!(month.len(), 30);
    assert_eq!(month[19]["spend_usd_micros"], OLD_COST);

    for days in [0, 31] {
        let refused = call(&app, "captcha.stats", json!({"days": days})).await;
        assert_eq!(invalid_field(refused), "days");
    }
}

fn bucket_at(history: &Value, ts: i64) -> Value {
    let bucket_ms = history["bucket_ms"].as_i64().unwrap();

    history["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bucket| {
            let start = bucket["start_ms"].as_i64().unwrap();
            start <= ts && ts < start + bucket_ms
        })
        .unwrap()
        .clone()
}

#[tokio::test]
async fn usage_history_counts_overlapping_runs_by_engine() {
    let (_dir, app) = scratch_app().await;
    let now = now_ms();
    let camoufox = Build {
        id: BuildId::new(),
        source: BuildSource::Camoufox,
        engine: Engine::Camoufox,
        name: "Camoufox".to_string(),
        version: "1.0".to_string(),
        channel: None,
        path: PathBuf::from("/nonexistent/camoufox"),
        state: BuildState::Installed,
        sha256: None,
        size_bytes: 0,
        capabilities: Vec::new(),
        installed_at: None,
        last_referenced_at: None,
    };

    let mut stopped = instance_row(InstanceState::Stopped, None);
    stopped.started_at = Some(now - 5 * HOUR_MS);
    stopped.ended_at = Some(now - 3 * HOUR_MS);
    let mut long_ago = instance_row(InstanceState::Stopped, None);
    long_ago.started_at = Some(now - 30 * 24 * HOUR_MS);
    long_ago.ended_at = Some(now - 29 * 24 * HOUR_MS);
    let mut fox = instance_row(InstanceState::Running, None);
    fox.build_id = Some(camoufox.id);
    fox.started_at = Some(now - HOUR_MS / 2);
    let mut live = instance_row(InstanceState::Running, None);
    live.kind = InstanceKind::Live;
    live.started_at = Some(now - HOUR_MS / 2);
    let mut never_started = instance_row(InstanceState::Failed, None);
    never_started.ended_at = Some(now);
    app.store
        .run(move |c| {
            build::insert(c, &camoufox)?;
            [stopped, long_ago, fox, live, never_started]
                .iter()
                .try_for_each(|row| instance::insert(c, row))
        })
        .await
        .unwrap();

    let day = call(&app, "usage.history", json!({"window": "1d"}))
        .await
        .unwrap();

    assert_eq!(day["window"], "1d");
    assert_eq!(day["bucket_ms"], HOUR_MS);
    assert_eq!(day["buckets"].as_array().unwrap().len(), 24);
    assert_eq!(
        bucket_at(&day, now - 4 * HOUR_MS),
        json!({"start_ms": bucket_at(&day, now - 4 * HOUR_MS)["start_ms"], "chromium": 1, "camoufox": 0, "attached": 0, "traffic_bytes": 0})
    );
    assert_eq!(bucket_at(&day, now - 8 * HOUR_MS)["chromium"], 0);
    let current = bucket_at(&day, now);
    assert_eq!(current["camoufox"], 1);
    assert_eq!(current["attached"], 1);
    assert_eq!(current["chromium"], 0);

    let week = call(&app, "usage.history", json!({"window": "7d"}))
        .await
        .unwrap();

    assert_eq!(week["bucket_ms"], 6 * HOUR_MS);
    assert_eq!(week["buckets"].as_array().unwrap().len(), 28);
    assert_eq!(bucket_at(&week, now - 4 * HOUR_MS)["chromium"], 1);
    let total: i64 = week["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bucket| bucket["chromium"].as_i64().unwrap())
        .sum();
    assert!((1..=2).contains(&total), "{week}");

    let defaulted = call(&app, "usage.history", json!({})).await.unwrap();
    assert_eq!(defaulted["window"], "1d");
    let refused = call(&app, "usage.history", json!({"window": "2d"})).await;
    assert_eq!(invalid_field(refused), "window");
}

#[tokio::test]
async fn usage_history_reports_peak_concurrency_not_run_count() {
    let (_dir, app) = scratch_app().await;
    let now = now_ms();
    let serial = (now / HOUR_MS - 10) * HOUR_MS;
    let parallel = serial - 2 * HOUR_MS;
    let ran = |from: i64, to: i64| {
        let mut row = instance_row(InstanceState::Stopped, None);
        row.started_at = Some(from);
        row.ended_at = Some(to);
        row
    };
    let rows = [
        ran(serial + 1_000, serial + 2_000),
        ran(serial + 2_000, serial + 3_000),
        ran(serial + 3_000, serial + 4_000),
        ran(serial + 3_500, serial + 3_500),
        ran(parallel + 1_000, parallel + 5_000),
        ran(parallel + 2_000, parallel + 6_000),
    ];
    app.store
        .run(move |c| rows.iter().try_for_each(|row| instance::insert(c, row)))
        .await
        .unwrap();

    let day = call(&app, "usage.history", json!({"window": "1d"}))
        .await
        .unwrap();

    assert_eq!(bucket_at(&day, serial)["chromium"], 1);
    assert_eq!(bucket_at(&day, parallel)["chromium"], 2);
}

#[tokio::test]
async fn usage_traffic_sums_ended_runs_per_proxy_and_history_buckets_them() {
    let (_dir, app) = scratch_app().await;
    let now = now_ms();
    let (first, second) = (ProxyId::new(), ProxyId::new());
    let ended = |proxy: ProxyId, at: i64, bytes: Option<(u64, u64)>| {
        let mut row = instance_row(InstanceState::Stopped, None);
        row.started_at = Some(at - 1_000);
        row.ended_at = Some(at);
        row.proxy_id = Some(proxy);
        row.traffic = bytes.map(|(sent_bytes, received_bytes)| Traffic {
            sent_bytes,
            received_bytes,
        });
        row
    };
    let rows = [
        ended(first, now - 4 * HOUR_MS, Some((100, 1_000))),
        ended(first, now - 2 * HOUR_MS, Some((200, 2_000))),
        ended(second, now - HOUR_MS, Some((5, 50))),
        ended(second, now - HOUR_MS, None),
        ended(second, now - 20 * 24 * HOUR_MS, Some((7, 70))),
    ];
    app.store
        .run(move |c| rows.iter().try_for_each(|row| instance::insert(c, row)))
        .await
        .unwrap();

    let week = call(&app, "usage.traffic", json!({"group": "proxy", "days": 7}))
        .await
        .unwrap();
    let items = week["items"].as_array().unwrap();

    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], first.to_string());
    assert_eq!(items[0]["sent_bytes"], 300);
    assert_eq!(items[0]["received_bytes"], 3_000);
    assert_eq!(items[0]["runs"], 2);
    assert_eq!(items[1]["id"], second.to_string());
    assert_eq!(items[1]["sent_bytes"], 5);
    assert_eq!(items[1]["received_bytes"], 50);
    assert_eq!(items[1]["runs"], 1);

    let month = call(&app, "usage.traffic", json!({"group": "proxy", "days": 30}))
        .await
        .unwrap();
    assert_eq!(month["items"][1]["sent_bytes"], 12);
    assert_eq!(month["items"][1]["runs"], 2);

    let profiles = call(&app, "usage.traffic", json!({"group": "profile"}))
        .await
        .unwrap();
    assert_eq!(profiles["items"].as_array().unwrap().len(), 0);
    for days in [0, 91] {
        let refused = call(
            &app,
            "usage.traffic",
            json!({"group": "proxy", "days": days}),
        )
        .await;
        assert_eq!(invalid_field(refused), "days");
    }

    let day = call(&app, "usage.history", json!({"window": "1d"}))
        .await
        .unwrap();
    assert_eq!(bucket_at(&day, now - 4 * HOUR_MS)["traffic_bytes"], 1_100);
    assert_eq!(bucket_at(&day, now - 2 * HOUR_MS)["traffic_bytes"], 2_200);
    assert_eq!(bucket_at(&day, now - HOUR_MS)["traffic_bytes"], 55);
}

async fn balance_reply(State(hits): State<Arc<AtomicU32>>) -> Json<Value> {
    let hit = hits.fetch_add(1, Ordering::SeqCst) + 1;

    Json(json!({"errorId": 0, "balance": hit}))
}

#[tokio::test]
async fn captcha_balance_is_cached_until_refresh_or_key_change() {
    let hits = Arc::new(AtomicU32::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_url = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/getBalance", post(balance_reply))
        .with_state(Arc::clone(&hits));
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(
        dir.path(),
        &[("BRR_CAPTCHA__PROVIDER_URL", provider_url.as_str())],
    ))
    .await
    .unwrap();
    call(&app, "captcha.settings_set", json!({"api_key": API_KEY}))
        .await
        .unwrap();

    let first = call(&app, "captcha.balance", json!({})).await.unwrap();
    let cached = call(&app, "captcha.balance", json!({})).await.unwrap();

    assert_eq!(first["balance_usd_micros"], 1_000_000);
    assert!(first["fetched_at"].as_i64().unwrap() > 0);
    assert_eq!(cached, first);
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let refreshed = call(&app, "captcha.balance", json!({"refresh": true}))
        .await
        .unwrap();

    assert_eq!(refreshed["balance_usd_micros"], 2_000_000);
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    call(&app, "captcha.settings_set", json!({"api_key": OTHER_KEY}))
        .await
        .unwrap();
    let after_key_change = call(&app, "captcha.balance", json!({})).await.unwrap();

    assert_eq!(after_key_change["balance_usd_micros"], 3_000_000);
    assert_eq!(hits.load(Ordering::SeqCst), 3);

    call(&app, "captcha.settings_set", json!({"daily_solves": 10}))
        .await
        .unwrap();
    call(&app, "captcha.balance", json!({})).await.unwrap();

    assert_eq!(hits.load(Ordering::SeqCst), 3);
}
