use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

const CAUSE_PREFIX: &str = "TYPE_";
const MS_PER_SECOND: f64 = 1000.0;

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// Juggler names the content policy kind that caused a request (`TYPE_IMAGE`); the shared network
// ring and `verdict` read Chromium's resource type names.
pub(super) fn resource_type(cause: &str) -> &'static str {
    match cause.strip_prefix(CAUSE_PREFIX).unwrap_or(cause) {
        "DOCUMENT" | "SUBDOCUMENT" | "REFRESH" | "SAVEAS_DOWNLOAD" => "Document",
        "SCRIPT" => "Script",
        "IMAGE" | "IMAGESET" => "Image",
        "STYLESHEET" => "Stylesheet",
        "FONT" => "Font",
        "MEDIA" => "Media",
        "XMLHTTPREQUEST" => "XHR",
        "FETCH" => "Fetch",
        "WEBSOCKET" => "WebSocket",
        "PING" | "BEACON" | "CSP_REPORT" => "Ping",
        _ => "Other",
    }
}

// Juggler sends headers as a list of {name, value}; repeated names are joined as HTTP does.
pub(super) fn header_map(headers: &Value) -> Map<String, Value> {
    let mut map = Map::new();
    for header in headers.as_array().into_iter().flatten() {
        let (Some(name), Some(value)) = (header["name"].as_str(), header["value"].as_str()) else {
            continue;
        };
        match map.get_mut(name) {
            Some(Value::String(joined)) => {
                joined.push_str(", ");
                joined.push_str(value);
            }
            _ => {
                map.insert(name.to_owned(), json!(value));
            }
        }
    }

    map
}

fn mime_of(headers: &Map<String, Value>) -> &str {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .and_then(|(_, value)| value.as_str())
        .and_then(|value| value.split(';').next())
        .map_or("", str::trim)
}

// The Juggler Network, Runtime and Page events the shared rings read, as the CDP events they
// were written for. Juggler stamps no event with a time, so arrival time stands in.
pub(super) fn translate(method: &str, params: &Value) -> Option<(&'static str, Value)> {
    let now = now_secs();

    Some(match method {
        "Network.requestWillBeSent" => (
            "Network.requestWillBeSent",
            json!({
                "requestId": params["requestId"],
                "timestamp": now,
                "wallTime": now,
                "request": {
                    "url": params["url"],
                    "method": params["method"],
                    "headers": header_map(&params["headers"]),
                },
                "type": resource_type(params["cause"].as_str().unwrap_or_default()),
            }),
        ),
        "Network.responseReceived" => {
            let headers = header_map(&params["headers"]);

            (
                "Network.responseReceived",
                json!({
                    "requestId": params["requestId"],
                    "response": {
                        "status": params["status"],
                        "statusText": params["statusText"],
                        "mimeType": mime_of(&headers),
                        "headers": headers,
                    },
                }),
            )
        }
        "Network.requestFinished" => (
            "Network.loadingFinished",
            json!({
                "requestId": params["requestId"],
                "timestamp": now,
                "encodedDataLength": params["transferSize"],
            }),
        ),
        "Network.requestFailed" => (
            "Network.loadingFailed",
            json!({
                "requestId": params["requestId"],
                "timestamp": now,
                "errorText": params["errorCode"],
            }),
        ),
        "Runtime.console" => {
            let kind = match params["type"].as_str().unwrap_or("log") {
                "warn" => "warning",
                other => other,
            };

            (
                "Runtime.consoleAPICalled",
                json!({
                    "type": kind,
                    "args": params["args"],
                    "timestamp": now * MS_PER_SECOND,
                    "stackTrace": { "callFrames": [{
                        "url": params["location"]["url"],
                        "lineNumber": zero_based(&params["location"]["lineNumber"]),
                    }] },
                }),
            )
        }
        "Page.uncaughtError" => (
            "Runtime.exceptionThrown",
            json!({
                "timestamp": now * MS_PER_SECOND,
                "exceptionDetails": {
                    "text": params["message"],
                    "url": params["location"]["url"],
                    "lineNumber": zero_based(&params["location"]["lineNumber"]),
                    "exception": { "description": params["message"] },
                },
            }),
        ),
        _ => return None,
    })
}

// Firefox counts lines from 1; the shared rings add the 1 CDP leaves out.
fn zero_based(line: &Value) -> u64 {
    line.as_u64().unwrap_or(1).saturating_sub(1)
}
