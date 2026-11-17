use serde_json::{Value, json};
use test_case::test_case;

use super::downloads::{content_length, is_attachment};
use super::input::modifiers;
use super::network::{resource_type, translate as translate_network};
use super::session::{from_serialized, remote_result, translate};
use crate::page::{JS_DIALOG_CLOSED, JS_DIALOG_OPENING};

#[test]
fn context_events_become_cdp_contexts() {
    let created = json!({
        "executionContextId": "id-7",
        "auxData": { "frameId": "mainframe-9", "name": "__brr_utility__" },
    });

    let (method, params) = translate("Runtime.executionContextCreated", &created).unwrap();

    assert_eq!(method, "Runtime.executionContextCreated");
    assert_eq!(params["context"]["id"], 7);
    assert_eq!(params["context"]["name"], "__brr_utility__");
    assert_eq!(params["context"]["auxData"]["frameId"], "mainframe-9");
    assert_eq!(params["context"]["auxData"]["isDefault"], false);
}

#[test]
fn the_unnamed_context_is_the_default() {
    let created =
        json!({ "executionContextId": "id-3", "auxData": { "frameId": "f", "name": "" } });

    let (_, params) = translate("Runtime.executionContextCreated", &created).unwrap();

    assert_eq!(params["context"]["auxData"]["isDefault"], true);
}

#[test]
fn a_malformed_context_id_is_ignored() {
    let created = json!({ "executionContextId": "7", "auxData": { "frameId": "f", "name": "" } });

    assert!(translate("Runtime.executionContextCreated", &created).is_none());
}

#[test]
fn dialogs_become_javascript_dialog_events() {
    let opened = json!({ "dialogId": "d1", "type": "prompt", "message": "m", "defaultValue": "x" });

    let (method, params) = translate("Page.dialogOpened", &opened).unwrap();
    assert_eq!(method, JS_DIALOG_OPENING);
    assert_eq!(params["defaultPrompt"], "x");
    assert_eq!(
        translate("Page.dialogClosed", &json!({})).unwrap().0,
        JS_DIALOG_CLOSED
    );
}

#[test]
fn modifier_bits_follow_firefox() {
    // CDP: Alt 1, Control 2, Meta 4, Shift 8. Firefox: Alt 1, Control 2, Shift 4, Meta 8.
    assert_eq!(modifiers(&json!(1 | 2)), 1 | 2);
    assert_eq!(modifiers(&json!(4)), 8);
    assert_eq!(modifiers(&json!(8)), 4);
    assert_eq!(modifiers(&json!(4 | 8)), 12);
}

#[test_case("TYPE_IMAGE", "Image")]
#[test_case("TYPE_IMAGESET", "Image")]
#[test_case("TYPE_STYLESHEET", "Stylesheet")]
#[test_case("TYPE_XMLHTTPREQUEST", "XHR")]
#[test_case("TYPE_FETCH", "Fetch")]
#[test_case("TYPE_SUBDOCUMENT", "Document")]
#[test_case("TYPE_BEACON", "Ping")]
#[test_case("TYPE_SOMETHING_NEW", "Other")]
fn causes_become_chromium_resource_types(cause: &str, expected: &str) {
    assert_eq!(resource_type(cause), expected);
}

#[test]
fn requests_become_cdp_requests_with_header_maps() {
    let sent = json!({
        "requestId": "r1",
        "url": "http://a/x.png",
        "method": "GET",
        "cause": "TYPE_IMAGE",
        "headers": [{ "name": "Accept", "value": "a" }, { "name": "Accept", "value": "b" }],
    });

    let (method, params) = translate_network("Network.requestWillBeSent", &sent).unwrap();

    assert_eq!(method, "Network.requestWillBeSent");
    assert_eq!(params["type"], "Image");
    assert_eq!(params["request"]["headers"]["Accept"], "a, b");
}

#[test]
fn responses_carry_the_mime_type_of_their_content_type() {
    let received = json!({
        "requestId": "r1",
        "status": 200,
        "statusText": "OK",
        "headers": [{ "name": "Content-Type", "value": "text/html; charset=utf-8" }],
    });

    let (_, params) = translate_network("Network.responseReceived", &received).unwrap();

    assert_eq!(params["response"]["mimeType"], "text/html");
}

#[test]
fn firefox_lines_start_at_one_and_cdp_lines_at_zero() {
    let console = json!({
        "type": "warn",
        "args": [],
        "location": { "url": "http://a/", "lineNumber": 3 },
    });

    let (method, params) = translate_network("Runtime.console", &console).unwrap();

    assert_eq!(method, "Runtime.consoleAPICalled");
    assert_eq!(params["type"], "warning");
    assert_eq!(params["stackTrace"]["callFrames"][0]["lineNumber"], 2);
}

#[test]
fn failures_and_errors_map_to_their_cdp_events() {
    let failed = json!({ "requestId": "r1", "errorCode": "blockedbyclient" });
    let (method, params) = translate_network("Network.requestFailed", &failed).unwrap();
    assert_eq!(method, "Network.loadingFailed");
    assert_eq!(params["errorText"], "blockedbyclient");

    let thrown = json!({ "message": "boom", "location": { "url": "u", "lineNumber": 1 } });
    let (method, params) = translate_network("Page.uncaughtError", &thrown).unwrap();
    assert_eq!(method, "Runtime.exceptionThrown");
    assert_eq!(
        params["exceptionDetails"]["exception"]["description"],
        "boom"
    );
}

#[test]
fn download_headers_are_read_case_insensitively() {
    let headers = json!([
        { "name": "Content-Disposition", "value": "Attachment; filename=a.bin" },
        { "name": "content-length", "value": "42" },
    ]);
    let inline = json!([{ "name": "Content-Length", "value": "x" }]);

    assert!(is_attachment(&headers));
    assert_eq!(content_length(&headers), Some(42));
    assert!(!is_attachment(&inline));
    assert_eq!(content_length(&inline), None);
}

#[test]
fn the_page_worlds_serialization_becomes_json() {
    let serialized = json!({ "o": [
        { "k": "n", "v": 1 },
        { "k": "list", "v": { "a": [{ "v": "null" }, "x"], "id": 2 } },
        { "k": "gone", "v": { "v": "undefined" } },
    ], "id": 1 });

    assert_eq!(
        from_serialized(&serialized),
        Some(json!({ "n": 1, "list": [null, "x"] }))
    );
    assert_eq!(from_serialized(&json!({ "v": "undefined" })), None);
}

// Juggler drops `value: undefined` from the wire, so undefined arrives as an empty result.
#[test_case(json!({ "result": {} }), json!({ "type": "undefined" }) ; "undefined")]
#[test_case(json!({}), json!({ "type": "undefined" }) ; "no result")]
#[test_case(json!({ "result": { "value": null } }), json!({ "value": null }) ; "null")]
#[test_case(json!({ "result": { "value": false } }), json!({ "value": false }) ; "a value")]
#[test_case(
    json!({ "result": { "objectId": "o1", "type": "object" } }),
    json!({ "objectId": "o1", "type": "object" }) ;
    "an object"
)]
fn call_results_keep_undefined_apart_from_values(response: Value, expected: Value) {
    assert_eq!(remote_result(response).unwrap(), expected);
}
