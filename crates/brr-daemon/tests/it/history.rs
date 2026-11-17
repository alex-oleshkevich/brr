#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{path::Path, sync::Arc};

use brr_core::{
    BuildChoice, Display, EndReason, Error, InstanceState, ProfileKind, ProfileSettings,
};
use brr_daemon::{App, ops};
use brr_store::instance;
use serde_json::{Value, json};
use test_case::test_case;

use common::{ctx, instance_row, register_build, scratch_app};

const NO_BROWSER: &str = "/nonexistent/chrome";
const TEMPLATE: &str = "history";
const PROFILE: &str = "remembered";
const EDITED_LOCALE: &str = "de-DE";
const TEMPLATE_ID: &str = "01JD0000000000000000000000";
const ENDED_STOPPED_AT: i64 = 2_000;
const ENDED_FAILED_AT: i64 = 3_000;
const QUEUED_RUNNING_AT: i64 = 4_000;
const BETWEEN_ENDS: i64 = 2_500;
const AFTER_FAILED: i64 = 3_500;
const OLDEST_ULID: &str = "00000000000000000000000000";

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    ops::dispatch(ctx(app), ops::find(name).unwrap(), input).await
}

#[tokio::test]
async fn launch_keeps_what_it_ran_with_after_a_profile_edit() {
    let (_dir, app) = scratch_app().await;
    let build_id = register_build(&app, Path::new(NO_BROWSER)).await;
    let settings = ProfileSettings {
        build: BuildChoice::BuildId(build_id),
        display: Display::Headless,
        ..ProfileSettings::default()
    };
    let template = call(
        &app,
        "template.create",
        json!({"name": TEMPLATE, "settings": &settings}),
    )
    .await
    .unwrap();
    call(
        &app,
        "profile.create",
        json!({"name": PROFILE, "template": TEMPLATE, "kind": ProfileKind::Ephemeral}),
    )
    .await
    .unwrap();

    let launched = call(&app, "instance.launch", json!({"profile": PROFILE})).await;
    let mut edited = settings.clone();
    edited.locale = Some(EDITED_LOCALE.to_owned());
    call(
        &app,
        "profile.update",
        json!({"profile": PROFILE, "settings": edited}),
    )
    .await
    .unwrap();
    let listed = call(&app, "instance.list", json!({"profile": PROFILE}))
        .await
        .unwrap();
    let ran = call(
        &app,
        "instance.get",
        json!({"instance_id": listed["instances"][0]["id"]}),
    )
    .await
    .unwrap();

    assert!(launched.is_err(), "{launched:?}");
    assert_eq!(ran["settings"], serde_json::to_value(&settings).unwrap());
    assert_eq!(ran["template_id"], template["id"]);
    assert_eq!(ran["template_name"], TEMPLATE);
    assert_eq!(ran["extensions"], json!([]));
    assert_eq!(ran["proxy_id"], Value::Null);
}

async fn add_runs(app: &Arc<App>) {
    let mut stopped = instance_row(InstanceState::Stopped, None);
    stopped.profile_name = Some("stopped".to_owned());
    stopped.template_id = Some(TEMPLATE_ID.parse().unwrap());
    stopped.template_name = Some(TEMPLATE.to_owned());
    stopped.end_reason = Some(EndReason::Stop);
    stopped.ended_at = Some(ENDED_STOPPED_AT);

    let mut failed = instance_row(InstanceState::Failed, None);
    failed.profile_name = Some("failed".to_owned());
    failed.end_reason = Some(EndReason::LaunchError);
    failed.ended_at = Some(ENDED_FAILED_AT);

    let mut running = instance_row(InstanceState::Running, None);
    running.profile_name = Some("running".to_owned());
    running.template_id = Some(TEMPLATE_ID.parse().unwrap());
    running.template_name = Some(TEMPLATE.to_owned());
    running.queued_at = QUEUED_RUNNING_AT;

    app.store
        .run(move |c| {
            [stopped, failed, running]
                .iter()
                .try_for_each(|row| instance::insert(c, row))
        })
        .await
        .unwrap();
}

fn names(listed: &Value) -> Vec<String> {
    let mut names: Vec<String> = listed["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["profile_name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();

    names
}

#[test_case(json!({"template": TEMPLATE}), &["running", "stopped"]; "template by name")]
#[test_case(json!({"template": TEMPLATE_ID}), &["running", "stopped"]; "template by id")]
#[test_case(json!({"end_reason": EndReason::Stop}), &["stopped"]; "end reason")]
#[test_case(json!({"since": BETWEEN_ENDS}), &["failed", "running"]; "since")]
#[test_case(json!({"until": BETWEEN_ENDS}), &["stopped"]; "until")]
#[test_case(json!({"since": BETWEEN_ENDS, "until": AFTER_FAILED}), &["failed"]; "window")]
#[tokio::test]
async fn instance_list_filters(filter: Value, expected: &[&str]) {
    let (_dir, app) = scratch_app().await;
    add_runs(&app).await;

    let listed = call(&app, "instance.list", filter).await.unwrap();

    assert_eq!(names(&listed), expected);
}

#[tokio::test]
async fn instance_list_pages_with_a_cursor() {
    let (_dir, app) = scratch_app().await;
    add_runs(&app).await;
    let all = call(&app, "instance.list", json!({})).await.unwrap();

    let mut paged = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let page = call(&app, "instance.list", json!({"limit": 1, "cursor": cursor}))
            .await
            .unwrap();
        paged.extend(
            page["instances"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["id"].clone()),
        );
        cursor = page["next_cursor"].clone();
        if cursor.is_null() {
            break;
        }
    }

    let ids: Vec<Value> = all["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].clone())
        .collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(paged, ids);
}

#[test_case("ZZZ"; "garbage")]
#[test_case("01JD00000000000000000000000"; "too long")]
#[tokio::test]
async fn instance_list_refuses_a_malformed_cursor(cursor: &str) {
    let (_dir, app) = scratch_app().await;
    add_runs(&app).await;

    let listed = call(&app, "instance.list", json!({"cursor": cursor})).await;

    let Err(Error::InvalidArgument { field, message }) = listed else {
        panic!("expected InvalidArgument, got {listed:?}");
    };
    assert_eq!(field, "cursor");
    assert!(message.contains("next_cursor"), "{message}");
}

// A well-formed cursor older than every run is past the last page, not a reason to start over.
#[tokio::test]
async fn instance_list_pages_from_an_unknown_cursor_without_restarting() {
    let (_dir, app) = scratch_app().await;
    add_runs(&app).await;

    let listed = call(&app, "instance.list", json!({"cursor": OLDEST_ULID}))
        .await
        .unwrap();

    assert_eq!(listed["instances"], json!([]));
    assert_eq!(listed["next_cursor"], Value::Null);
}

#[tokio::test]
async fn instance_list_filter_errors_say_what_to_pass() {
    let (_dir, app) = scratch_app().await;
    add_runs(&app).await;

    let template = call(&app, "instance.list", json!({"template": "nobody"})).await;
    let state = call(&app, "instance.list", json!({"state": "running,asleep"})).await;
    let since = call(&app, "instance.list", json!({"since": "yesterday"})).await;

    assert!(
        matches!(template, Err(Error::NotFound { .. })),
        "{template:?}"
    );
    let Err(Error::InvalidArgument { field, message }) = state else {
        panic!("expected InvalidArgument, got {state:?}");
    };
    assert_eq!(field, "state");
    assert!(message.contains("`stopped`"), "{message}");
    let Err(Error::InvalidArgument { field, message }) = since else {
        panic!("expected InvalidArgument, got {since:?}");
    };
    assert_eq!(field, "since");
    assert!(message.contains("epoch milliseconds"), "{message}");
}

#[tokio::test]
async fn instance_get_with_a_profile_name_points_to_the_list() {
    let (_dir, app) = scratch_app().await;
    call(
        &app,
        "profile.create",
        json!({"name": PROFILE, "template": "chromium-headless"}),
    )
    .await
    .unwrap();

    let got = call(&app, "instance.get", json!({"instance_id": PROFILE})).await;

    let Err(Error::InvalidArgument { field, message }) = got else {
        panic!("expected InvalidArgument, got {got:?}");
    };
    assert_eq!(field, "instance_id");
    assert!(
        message.contains(&format!("instance list --profile {PROFILE}")),
        "{message}"
    );
}
