use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use brr_cdp::SessionId;
use brr_core::{Cookie, Error, SameSite, UtcDateTime};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::debug;

use super::{Page, PageState, Session, capped};
use crate::chromium::fetch::{self, RequestRules};

#[cfg(test)]
mod tests;

pub const HAR_MAX_ENTRIES: usize = 10_000;

const MAX_CONSOLE_TEXT_CHARS: usize = 4096;
// URLs and header values come from the page; a data: URL can be megabytes.
const MAX_URL_CHARS: usize = 2048;
const MAX_HEADER_VALUE_CHARS: usize = 1024;
const HAR_VERSION: &str = "1.2";
const HAR_CREATOR: &str = "brr";
const HAR_DEFAULT_HTTP_VERSION: &str = "HTTP/1.1";
const HAR_UNKNOWN_SIZE: i64 = -1;
const HAR_FIELD: &str = "har";
const AREA_FIELD: &str = "area";
const REQUEST_STARTED: &str = "Network.requestWillBeSent";
const RESPONSE_RECEIVED: &str = "Network.responseReceived";
const REQUEST_FINISHED: &str = "Network.loadingFinished";
const REQUEST_FAILED: &str = "Network.loadingFailed";
const CONSOLE_CALLED: &str = "Runtime.consoleAPICalled";
const EXCEPTION_THROWN: &str = "Runtime.exceptionThrown";
const LOG_ENTRY: &str = "Log.entryAdded";
const MS_PER_SECOND: f64 = 1000.0;
// A frame without an origin (about:blank, data:) has no Web storage.
const STORAGE_SCRIPT: &str = "(() => {
    if (location.origin === 'null') return null;
    const storage = window.AREAStorage;
    BODY
})()";
const STORAGE_READ: &str = "return Object.fromEntries(Object.entries(storage));";
const STORAGE_CLEAR: &str = "storage.clear(); return true;";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StorageArea {
    Local,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ConsoleMessage {
    pub level: String,
    pub text: String,
    pub url: Option<String>,
    pub line: Option<u32>,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NetworkEntry {
    pub request_id: String,
    pub method: String,
    pub url: String,
    pub resource_type: String,
    pub status: Option<u16>,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub failure: Option<String>,
    pub started_at: i64,
    pub duration_ms: Option<u64>,
}

// Oldest entries are dropped once the cap is reached.
struct Ring<T> {
    items: VecDeque<T>,
    cap: usize,
}

impl<T> Ring<T> {
    fn new(cap: usize) -> Ring<T> {
        Ring {
            items: VecDeque::new(),
            cap,
        }
    }

    fn push(&mut self, item: T) {
        self.items.push_back(item);
        if self.items.len() > self.cap {
            self.items.pop_front();
        }
    }

    // Redirects reuse a request id, so the newest entry is the one that is still live.
    fn latest_mut(&mut self, matches: impl Fn(&T) -> bool) -> Option<&mut T> {
        self.items.iter_mut().rev().find(|item| matches(item))
    }
}

// A ring entry plus the monotonic start the duration is measured from.
struct Logged {
    entry: NetworkEntry,
    timestamp: f64,
}

// Requests in flight, by request id and the session that sent them, and since when there were
// none; the console and network rings and the HAR recording live beside them.
pub(super) struct Network {
    in_flight: HashMap<String, SessionId>,
    idle_since: Option<Instant>,
    console: Ring<ConsoleMessage>,
    requests: Ring<Logged>,
    har: Option<Ring<HarEntry>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestEvent {
    request_id: String,
    timestamp: f64,
    wall_time: f64,
    request: RequestPayload,
    #[serde(rename = "type")]
    resource_type: Option<String>,
    redirect_response: Option<ResponsePayload>,
}

#[derive(Deserialize)]
struct RequestPayload {
    url: String,
    method: String,
    headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseEvent {
    request_id: String,
    response: ResponsePayload,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponsePayload {
    status: u16,
    status_text: String,
    headers: BTreeMap<String, String>,
    mime_type: String,
    protocol: Option<String>,
    timing: Option<Timing>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Timing {
    request_time: f64,
    send_start: f64,
    send_end: f64,
    receive_headers_end: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinishedEvent {
    request_id: String,
    timestamp: f64,
    encoded_data_length: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FailedEvent {
    request_id: String,
    timestamp: f64,
    error_text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConsoleEvent {
    #[serde(rename = "type")]
    kind: String,
    args: Vec<Value>,
    timestamp: f64,
    stack_trace: Option<StackTrace>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StackTrace {
    call_frames: Vec<CallFrame>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallFrame {
    url: String,
    line_number: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExceptionEvent {
    timestamp: f64,
    exception_details: ExceptionDetails,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExceptionDetails {
    text: String,
    line_number: Option<u32>,
    url: Option<String>,
    exception: Option<ExceptionObject>,
}

#[derive(Deserialize)]
struct ExceptionObject {
    description: Option<String>,
}

#[derive(Deserialize)]
struct LogEvent {
    entry: LogEntry,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    level: String,
    text: String,
    timestamp: f64,
    url: Option<String>,
    line_number: Option<u32>,
}

#[derive(Serialize)]
struct Header {
    name: String,
    value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HarRequest {
    method: String,
    url: String,
    http_version: String,
    cookies: Vec<Value>,
    headers: Vec<Header>,
    query_string: Vec<Header>,
    headers_size: i64,
    body_size: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HarContent {
    size: u64,
    mime_type: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HarResponse {
    status: u16,
    status_text: String,
    http_version: String,
    cookies: Vec<Value>,
    headers: Vec<Header>,
    content: HarContent,
    #[serde(rename = "redirectURL")]
    redirect_url: String,
    headers_size: i64,
    body_size: i64,
}

#[derive(Serialize)]
struct HarTimings {
    blocked: f64,
    send: f64,
    wait: f64,
    receive: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HarEntry {
    #[serde(skip)]
    request_id: String,
    // Monotonic seconds of the request and of its timing's origin.
    #[serde(skip)]
    timestamp: f64,
    #[serde(skip)]
    request_time: f64,
    started_date_time: String,
    time: f64,
    request: HarRequest,
    response: HarResponse,
    cache: BTreeMap<String, String>,
    timings: HarTimings,
}

fn headers_of(headers: BTreeMap<String, String>) -> Vec<Header> {
    headers
        .into_iter()
        .map(|(name, value)| Header {
            name,
            value: capped(&value, MAX_HEADER_VALUE_CHARS),
        })
        .collect()
}

// Milliseconds since the epoch as the ISO 8601 UTC time HAR wants.
fn iso_time(unix_ms: i64) -> String {
    let time = UtcDateTime::from_unix_ms(unix_ms);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        time.year, time.month, time.day, time.hour, time.minute, time.second, time.millisecond
    )
}

fn epoch_ms(seconds: f64) -> i64 {
    (seconds * MS_PER_SECOND) as i64
}

fn har_response(response: ResponsePayload, redirect_url: String) -> HarResponse {
    HarResponse {
        status: response.status,
        status_text: response.status_text,
        http_version: response
            .protocol
            .unwrap_or_else(|| HAR_DEFAULT_HTTP_VERSION.to_owned()),
        cookies: Vec::new(),
        headers: headers_of(response.headers),
        content: HarContent {
            size: 0,
            mime_type: response.mime_type,
        },
        redirect_url: capped(&redirect_url, MAX_URL_CHARS),
        headers_size: HAR_UNKNOWN_SIZE,
        body_size: HAR_UNKNOWN_SIZE,
    }
}

// `wait` is the time to first byte: from the request's last byte sent to the response headers.
fn apply_timing(entry: &mut HarEntry, timing: &Timing) {
    entry.request_time = timing.request_time;
    entry.timings.blocked = timing.send_start.max(0.0);
    entry.timings.send = timing.send_end - timing.send_start;
    entry.timings.wait = timing.receive_headers_end - timing.send_end;
}

fn console_text(args: &[Value]) -> String {
    let parts: Vec<String> = args
        .iter()
        .map(|arg| match &arg["value"] {
            Value::String(text) => text.clone(),
            Value::Null => arg["description"]
                .as_str()
                .or_else(|| arg["type"].as_str())
                .unwrap_or_default()
                .to_owned(),
            value => value.to_string(),
        })
        .collect();

    capped(&parts.join(" "), MAX_CONSOLE_TEXT_CHARS)
}

impl Network {
    pub(super) fn new(console_buffer: usize, network_buffer: usize) -> Network {
        Network {
            in_flight: HashMap::new(),
            idle_since: Some(Instant::now()),
            console: Ring::new(console_buffer),
            requests: Ring::new(network_buffer),
            har: None,
        }
    }

    // Whether `apply` reads this method, so the event loop skips the rest cheaply.
    pub(super) fn tracks(method: &str) -> bool {
        matches!(
            method,
            REQUEST_STARTED
                | RESPONSE_RECEIVED
                | REQUEST_FINISHED
                | REQUEST_FAILED
                | CONSOLE_CALLED
                | EXCEPTION_THROWN
                | LOG_ENTRY
        )
    }

    pub(super) fn idle(&self, quiet: Duration) -> bool {
        self.idle_since
            .is_some_and(|since| since.elapsed() >= quiet)
    }

    // Params of an unexpected shape are ignored: a page cannot make the browser misreport.
    pub(super) fn apply(&mut self, session_id: &SessionId, method: &str, params: &Value) {
        match method {
            REQUEST_STARTED => {
                if let Ok(event) = RequestEvent::deserialize(params) {
                    self.request_started(session_id, event);
                }
            }
            RESPONSE_RECEIVED => {
                if let Ok(event) = ResponseEvent::deserialize(params) {
                    self.response_received(event);
                }
            }
            REQUEST_FINISHED => {
                if let Ok(event) = FinishedEvent::deserialize(params) {
                    self.finished(event);
                }
            }
            REQUEST_FAILED => {
                if let Ok(event) = FailedEvent::deserialize(params) {
                    self.failed(event);
                }
            }
            CONSOLE_CALLED => {
                if let Ok(event) = ConsoleEvent::deserialize(params) {
                    self.console_called(event);
                }
            }
            EXCEPTION_THROWN => {
                if let Ok(event) = ExceptionEvent::deserialize(params) {
                    self.exception_thrown(event);
                }
            }
            LOG_ENTRY => {
                if let Ok(event) = LogEvent::deserialize(params) {
                    self.log_entry(event.entry);
                }
            }
            _ => {}
        }
    }

    fn settle(&mut self, request_id: &str) {
        if self.in_flight.remove(request_id).is_some() && self.in_flight.is_empty() {
            self.idle_since = Some(Instant::now());
        }
    }

    fn request_started(&mut self, session_id: &SessionId, event: RequestEvent) {
        self.in_flight
            .insert(event.request_id.clone(), session_id.clone());
        self.idle_since = None;

        let started_at = epoch_ms(event.wall_time);
        let url = capped(&event.request.url, MAX_URL_CHARS);
        if let Some(redirect) = event.redirect_response {
            self.redirected(&event.request_id, &url, redirect, event.timestamp);
        }

        if let Some(har) = &mut self.har {
            har.push(HarEntry {
                request_id: event.request_id.clone(),
                timestamp: event.timestamp,
                request_time: 0.0,
                started_date_time: iso_time(started_at),
                time: 0.0,
                request: HarRequest {
                    method: event.request.method.clone(),
                    url: url.clone(),
                    http_version: HAR_DEFAULT_HTTP_VERSION.to_owned(),
                    cookies: Vec::new(),
                    headers: headers_of(event.request.headers),
                    query_string: Vec::new(),
                    headers_size: HAR_UNKNOWN_SIZE,
                    body_size: HAR_UNKNOWN_SIZE,
                },
                response: HarResponse {
                    status: 0,
                    status_text: String::new(),
                    http_version: HAR_DEFAULT_HTTP_VERSION.to_owned(),
                    cookies: Vec::new(),
                    headers: Vec::new(),
                    content: HarContent {
                        size: 0,
                        mime_type: String::new(),
                    },
                    redirect_url: String::new(),
                    headers_size: HAR_UNKNOWN_SIZE,
                    body_size: HAR_UNKNOWN_SIZE,
                },
                cache: BTreeMap::new(),
                timings: HarTimings {
                    blocked: 0.0,
                    send: 0.0,
                    wait: 0.0,
                    receive: 0.0,
                },
            });
        }

        self.requests.push(Logged {
            entry: NetworkEntry {
                request_id: event.request_id,
                method: event.request.method,
                url,
                resource_type: event.resource_type.unwrap_or_default(),
                status: None,
                mime: None,
                size: None,
                failure: None,
                started_at,
                duration_ms: None,
            },
            timestamp: event.timestamp,
        });
    }

    // The redirect's own response closes the entry the new request replaces; the redirect
    // is the end of that hop, so it is timed up to the new request.
    fn redirected(&mut self, request_id: &str, location: &str, response: ResponsePayload, ts: f64) {
        if let Some(logged) = self
            .requests
            .latest_mut(|l| l.entry.request_id == request_id)
        {
            logged.entry.status = Some(response.status);
            logged.entry.mime = Some(response.mime_type.clone());
            logged.entry.duration_ms = Some(elapsed_ms(logged.timestamp, ts) as u64);
        }
        if let Some(har) = &mut self.har
            && let Some(entry) = har.latest_mut(|e| e.request_id == request_id)
        {
            if let Some(timing) = &response.timing {
                apply_timing(entry, timing);
                entry.timings.receive =
                    (elapsed_ms(timing.request_time, ts) - timing.receive_headers_end).max(0.0);
            }
            entry.time = elapsed_ms(entry.timestamp, ts);
            entry.response = har_response(response, location.to_owned());
        }
    }

    fn response_received(&mut self, event: ResponseEvent) {
        let response = event.response;
        if let Some(logged) = self
            .requests
            .latest_mut(|l| l.entry.request_id == event.request_id)
        {
            logged.entry.status = Some(response.status);
            logged.entry.mime = Some(response.mime_type.clone());
        }
        if let Some(har) = &mut self.har
            && let Some(entry) = har.latest_mut(|e| e.request_id == event.request_id)
        {
            if let Some(timing) = &response.timing {
                apply_timing(entry, timing);
            }
            entry.response = har_response(response, String::new());
        }
    }

    fn finished(&mut self, event: FinishedEvent) {
        self.settle(&event.request_id);

        let size = event.encoded_data_length as u64;
        if let Some(logged) = self
            .requests
            .latest_mut(|l| l.entry.request_id == event.request_id)
        {
            logged.entry.size = Some(size);
            logged.entry.duration_ms = Some(elapsed_ms(logged.timestamp, event.timestamp) as u64);
        }
        if let Some(har) = &mut self.har
            && let Some(entry) = har.latest_mut(|e| e.request_id == event.request_id)
        {
            entry.response.content.size = size;
            entry.time = elapsed_ms(entry.timestamp, event.timestamp);
            if entry.request_time > 0.0 {
                let headers_end = entry.timings.blocked + entry.timings.send + entry.timings.wait;
                entry.timings.receive =
                    (elapsed_ms(entry.request_time, event.timestamp) - headers_end).max(0.0);
            }
        }
    }

    fn failed(&mut self, event: FailedEvent) {
        self.settle(&event.request_id);

        if let Some(logged) = self
            .requests
            .latest_mut(|l| l.entry.request_id == event.request_id)
        {
            logged.entry.failure = Some(event.error_text);
            logged.entry.duration_ms = Some(elapsed_ms(logged.timestamp, event.timestamp) as u64);
        }
        if let Some(har) = &mut self.har
            && let Some(entry) = har.latest_mut(|e| e.request_id == event.request_id)
        {
            entry.time = elapsed_ms(entry.timestamp, event.timestamp);
        }
    }

    fn console_called(&mut self, event: ConsoleEvent) {
        let frame = event
            .stack_trace
            .and_then(|trace| trace.call_frames.into_iter().next());
        self.console.push(ConsoleMessage {
            level: event.kind,
            text: console_text(&event.args),
            url: frame
                .as_ref()
                .map(|frame| capped(&frame.url, MAX_URL_CHARS)),
            line: frame.map(|frame| frame.line_number + 1),
            ts: event.timestamp as i64,
        });
    }

    fn exception_thrown(&mut self, event: ExceptionEvent) {
        let details = event.exception_details;
        let text = details
            .exception
            .and_then(|exception| exception.description)
            .unwrap_or(details.text);
        self.console.push(ConsoleMessage {
            level: "error".to_owned(),
            text: capped(&text, MAX_CONSOLE_TEXT_CHARS),
            url: details.url.map(|url| capped(&url, MAX_URL_CHARS)),
            line: details.line_number.map(|line| line + 1),
            ts: event.timestamp as i64,
        });
    }

    fn log_entry(&mut self, entry: LogEntry) {
        self.console.push(ConsoleMessage {
            level: entry.level,
            text: capped(&entry.text, MAX_CONSOLE_TEXT_CHARS),
            url: entry.url.map(|url| capped(&url, MAX_URL_CHARS)),
            line: entry.line_number.map(|line| line + 1),
            ts: entry.timestamp as i64,
        });
    }

    // Its pending requests will never report back.
    pub(super) fn session_detached(&mut self, session_id: &SessionId) {
        let before = self.in_flight.len();
        self.in_flight.retain(|_, session| session != session_id);
        if before > 0 && self.in_flight.is_empty() {
            self.idle_since = Some(Instant::now());
        }
    }

    pub(super) fn console_messages(&self) -> Vec<ConsoleMessage> {
        self.console.items.iter().cloned().collect()
    }

    // The URL and resource type of a request still in the ring.
    pub(super) fn request(&self, request_id: &str) -> Option<(String, String)> {
        self.requests
            .items
            .iter()
            .rev()
            .find(|logged| logged.entry.request_id == request_id)
            .map(|logged| (logged.entry.url.clone(), logged.entry.resource_type.clone()))
    }

    pub(super) fn network_log(&self) -> Vec<NetworkEntry> {
        self.requests
            .items
            .iter()
            .map(|logged| logged.entry.clone())
            .collect()
    }

    pub(super) fn har_start(&mut self) -> Result<(), Error> {
        if self.har.is_some() {
            return Err(har_error(
                "a HAR recording is already running; stop it first",
            ));
        }
        self.har = Some(Ring::new(HAR_MAX_ENTRIES));

        Ok(())
    }

    // The entries leave under the lock; serializing them is the caller's, after it is released.
    fn har_take(&mut self) -> Result<VecDeque<HarEntry>, Error> {
        self.har
            .take()
            .map(|har| har.items)
            .ok_or_else(|| har_error("no HAR recording is running; start one first"))
    }
}

#[derive(Serialize)]
struct HarDocument<'a> {
    log: HarLog<'a>,
}

#[derive(Serialize)]
struct HarLog<'a> {
    version: &'static str,
    creator: HarCreator,
    pages: [(); 0],
    entries: &'a VecDeque<HarEntry>,
}

#[derive(Serialize)]
struct HarCreator {
    name: &'static str,
    version: &'static str,
}

fn har_document(entries: &VecDeque<HarEntry>) -> HarDocument<'_> {
    HarDocument {
        log: HarLog {
            version: HAR_VERSION,
            creator: HarCreator {
                name: HAR_CREATOR,
                version: env!("CARGO_PKG_VERSION"),
            },
            pages: [],
            entries,
        },
    }
}

fn har_bytes(entries: &VecDeque<HarEntry>) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(&har_document(entries)).map_err(|error| Error::Internal {
        message: error.to_string(),
    })
}

fn redact_in_place(state: &PageState, text: &mut String) {
    *text = state.redact_owned(std::mem::take(text));
}

fn har_error(message: &str) -> Error {
    Error::InvalidArgument {
        field: HAR_FIELD.to_owned(),
        message: message.to_owned(),
    }
}

fn elapsed_ms(from: f64, to: f64) -> f64 {
    ((to - from) * MS_PER_SECOND).round().max(0.0)
}

#[derive(Deserialize)]
struct CdpCookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    expires: f64,
    #[serde(rename = "httpOnly")]
    http_only: bool,
    secure: bool,
    session: bool,
    #[serde(rename = "sameSite")]
    same_site: Option<SameSite>,
}

// The `cookies` array of Network.getCookies and Storage.getCookies.
pub(crate) fn parse_cookies(result: &mut Value) -> Result<Vec<Cookie>, Error> {
    let cookies: Vec<CdpCookie> =
        serde_json::from_value(result["cookies"].take()).map_err(|error| Error::Internal {
            message: error.to_string(),
        })?;

    Ok(cookies
        .into_iter()
        .map(|cookie| Cookie {
            name: cookie.name,
            value: cookie.value,
            domain: cookie.domain,
            path: cookie.path,
            expires: if cookie.session { -1.0 } else { cookie.expires },
            http_only: cookie.http_only,
            secure: cookie.secure,
            same_site: cookie.same_site.unwrap_or(SameSite::Lax),
        })
        .collect())
}

pub(crate) fn cookie_params(cookies: &[Cookie]) -> Vec<Value> {
    cookies
        .iter()
        .map(|cookie| {
            let mut params = json!({
                "name": cookie.name,
                "value": cookie.value,
                "domain": cookie.domain,
                "path": cookie.path,
                "httpOnly": cookie.http_only,
                "secure": cookie.secure,
                "sameSite": cookie.same_site,
            });
            if cookie.expires >= 0.0 {
                params["expires"] = json!(cookie.expires);
            }

            params
        })
        .collect()
}

// Whether a cookie would be sent to `url`: its domain covers the host and its path the path.
fn cookie_matches(cookie: &Cookie, url: &str) -> bool {
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host.rsplit_once(':').map_or(host, |(host, _)| host);
    let path = rest[authority_end..]
        .split(['?', '#'])
        .next()
        .filter(|path| !path.is_empty())
        .unwrap_or("/");

    let domain = cookie.domain.trim_start_matches('.');
    let host_matches = host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'));

    host_matches && path.starts_with(&cookie.path)
}

impl Page {
    // The browser context is the Tab's own: ephemeral contexts have separate cookies.
    async fn context_id(&self) -> Result<String, Error> {
        let page = self.session.chromium()?;
        let info = page
            .cdp
            .call(
                None,
                "Target.getTargetInfo",
                json!({ "targetId": page.target_id }),
            )
            .await?;

        info["targetInfo"]["browserContextId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::Internal {
                message: "Target.getTargetInfo: no browserContextId".to_owned(),
            })
    }

    // With `urls`, every cookie of the context that would be sent to one of them; without, the
    // cookies of the page's own URL.
    pub async fn cookies(&self, urls: &[String]) -> Result<Vec<Cookie>, Error> {
        if let Session::Camoufox(session) = &self.session {
            // Juggler lists a whole context, so the page's own URL is filtered here.
            let urls = if urls.is_empty() {
                vec![self.url().await?]
            } else {
                urls.to_vec()
            };
            let mut cookies = session.cookies().await?;
            cookies.retain(|cookie| urls.iter().any(|url| cookie_matches(cookie, url)));

            return Ok(cookies);
        }
        if urls.is_empty() {
            let page_url = self.url().await?;
            let mut result = self
                .session
                .cdp()?
                .call(
                    Some(self.session.main_session()),
                    "Network.getCookies",
                    json!({ "urls": [page_url] }),
                )
                .await?;

            return parse_cookies(&mut result);
        }

        let mut result = self
            .session
            .cdp()?
            .call(
                None,
                "Storage.getCookies",
                json!({ "browserContextId": self.context_id().await? }),
            )
            .await?;
        let mut cookies = parse_cookies(&mut result)?;
        cookies.retain(|cookie| urls.iter().any(|url| cookie_matches(cookie, url)));

        Ok(cookies)
    }

    pub async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), Error> {
        if let Session::Camoufox(session) = &self.session {
            return session.set_cookies(cookies).await;
        }
        let params = json!({
            "cookies": cookie_params(cookies),
            "browserContextId": self.context_id().await?,
        });
        self.session
            .cdp()?
            .call(None, "Storage.setCookies", params)
            .await?;

        Ok(())
    }

    pub async fn clear_cookies(&self) -> Result<(), Error> {
        if let Session::Camoufox(session) = &self.session {
            return session.clear_cookies().await;
        }
        let params = json!({ "browserContextId": self.context_id().await? });
        self.session
            .cdp()?
            .call(None, "Storage.clearCookies", params)
            .await?;

        Ok(())
    }

    // Runs in the utility world: isolated worlds share the origin's storage, and the page's own
    // scripts cannot replace `localStorage` under it.
    async fn run_storage(&self, area: StorageArea, body: &str) -> Result<Value, Error> {
        let area_name = match area {
            StorageArea::Local => "local",
            StorageArea::Session => "session",
        };
        let script = STORAGE_SCRIPT
            .replace("AREA", area_name)
            .replace("BODY", body);

        match self.evaluate_utility(&script).await? {
            Value::Null => Err(Error::InvalidArgument {
                field: AREA_FIELD.to_owned(),
                message: "page has no storage origin".to_owned(),
            }),
            value => Ok(value),
        }
    }

    pub async fn storage(&self, area: StorageArea) -> Result<BTreeMap<String, String>, Error> {
        let items = self.run_storage(area, STORAGE_READ).await?;

        serde_json::from_value(items).map_err(|error| Error::Internal {
            message: error.to_string(),
        })
    }

    pub async fn set_storage(
        &self,
        area: StorageArea,
        items: &BTreeMap<String, String>,
    ) -> Result<(), Error> {
        let body = format!(
            "for (const [key, value] of Object.entries({})) storage.setItem(key, value); return true;",
            json!(items)
        );
        self.run_storage(area, &body).await?;

        Ok(())
    }

    pub async fn clear_storage(&self, area: StorageArea) -> Result<(), Error> {
        self.run_storage(area, STORAGE_CLEAR).await?;

        Ok(())
    }

    pub fn console_messages(&self) -> Vec<ConsoleMessage> {
        let mut messages = self.state.network().console_messages();
        for message in &mut messages {
            redact_in_place(&self.state, &mut message.text);
            if let Some(url) = &mut message.url {
                redact_in_place(&self.state, url);
            }
        }

        messages
    }

    pub fn network_log(&self) -> Vec<NetworkEntry> {
        let mut entries = self.state.network().network_log();
        for entry in &mut entries {
            redact_in_place(&self.state, &mut entry.url);
        }

        entries
    }

    pub async fn set_request_rules(&self, rules: RequestRules) -> Result<(), Error> {
        self.state.set_rules(rules);

        self.apply_interception().await
    }

    // Fetch is enabled on every session of the Tab, so frames in other processes and workers
    // are covered too; sessions that attach later read the rules in init_session.
    pub(super) async fn apply_interception(&self) -> Result<(), Error> {
        let intercept = self.state.intercepting();
        if let Session::Camoufox(session) = &self.session {
            return session.set_intercepting(intercept).await;
        }

        // Every session is tried, so one dying session cannot leave the others unintercepted.
        let mut first_error = None;
        for session_id in self.state.sessions() {
            match fetch::set_enabled(self.session.cdp()?, &session_id, &self.state).await {
                Err(error) if intercept => {
                    first_error.get_or_insert(error);
                }
                // A session that refuses to disable keeps answering its paused requests, now
                // always with a continue.
                Err(error) => debug!(%error, "Fetch not disabled on a session"),
                Ok(()) => {}
            }
        }

        first_error.map_or(Ok(()), Err)
    }

    pub fn har_start(&self) -> Result<(), Error> {
        self.state.network().har_start()
    }

    pub fn har_stop(&self) -> Result<Vec<u8>, Error> {
        let entries = self.state.network().har_take()?;
        if !self.state.has_secrets() {
            return har_bytes(&entries);
        }

        let mut document =
            serde_json::to_value(har_document(&entries)).map_err(|error| Error::Internal {
                message: error.to_string(),
            })?;
        drop(entries);
        self.state.redact_value(&mut document);

        serde_json::to_vec(&document).map_err(|error| Error::Internal {
            message: error.to_string(),
        })
    }
}
