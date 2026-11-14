#![allow(clippy::unwrap_used)]

use crate::common;

use brr_core::Error;
use brr_daemon::ops;
use serde_json::{Value, json};
use test_case::test_case;

use common::{ctx, scratch_app};

const BATCH_OP: &str = "batch.run";
// With no Tab open and no tab_id, a Tab tool fails before it touches a browser.
const FAILING_STEP: &str = "get_metadata";

async fn run_batch(input: Value) -> Result<Value, Error> {
    let (_dir, app) = scratch_app().await;
    let op = ops::find(BATCH_OP).unwrap();

    ops::dispatch(ctx(&app), op, input).await
}

fn failing_steps(count: usize) -> Vec<Value> {
    (0..count)
        .map(|_| json!({ "tool": FAILING_STEP, "args": {} }))
        .collect()
}

#[test_case(json!({ "steps": [] }), "steps"; "empty")]
#[test_case(json!({ "steps": failing_steps(51) }), "steps"; "too many")]
#[test_case(json!({ "steps": [{ "tool": "no_such_tool" }] }), "steps[0].tool"; "unknown tool")]
#[test_case(json!({ "steps": [{ "tool": "client_create" }] }), "steps[0].tool"; "not a browser tool")]
#[test_case(json!({ "steps": [{ "tool": "batch_run" }] }), "steps[0].tool"; "nested batch")]
#[test_case(
    json!({ "steps": [{ "tool": FAILING_STEP, "args": { "client_id": "c" } }] }),
    "steps[0].args.client_id";
    "client id in a step"
)]
#[tokio::test]
async fn invalid_batch_is_rejected_before_any_step_runs(input: Value, expected_field: &str) {
    let Err(Error::InvalidArgument { field, .. }) = run_batch(input).await else {
        panic!("expected InvalidArgument");
    };

    assert_eq!(field, expected_field);
}

#[tokio::test]
async fn a_failing_step_stops_the_batch() {
    let output = run_batch(json!({ "steps": failing_steps(3) }))
        .await
        .unwrap();

    assert_eq!(output["total"], 3);
    assert_eq!(output["completed"], 0);
    assert_eq!(output["stopped"], "error");
    assert_eq!(output["results"].as_array().unwrap().len(), 1);
    assert_eq!(output["results"][0]["ok"], false);
    assert_eq!(output["results"][0]["step"], 0);
    assert_eq!(output["results"][0]["error"]["code"], "InvalidArgument");
}

#[tokio::test]
async fn continue_on_error_runs_every_step() {
    let output = run_batch(json!({ "steps": failing_steps(3), "continue_on_error": true }))
        .await
        .unwrap();

    assert_eq!(output["completed"], 0);
    assert_eq!(output["stopped"], Value::Null);
    assert_eq!(output["results"].as_array().unwrap().len(), 3);
}
