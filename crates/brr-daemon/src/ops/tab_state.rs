use std::collections::BTreeMap;

use brr_core::{
    ArtifactKind, Cookie, CookieInput, Error, Geolocation, SameSite, Size, Tab, TabId,
    capture::{ColorScheme, ResourceType, ReturnMode},
};
use brr_engine::{
    RequestRules,
    page::{ConsoleMessage, EmulateOptions, NetworkEntry, StorageArea},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    invalid, output,
    tab_capture::deliver,
    tool::{TabInput, tab_call, tab_op},
};
use crate::{app::Ctx, services::artifacts::Delivery};

const MAX_LATITUDE: f64 = 90.0;
const MAX_LONGITUDE: f64 = 180.0;
const DEFAULT_LIMIT: u32 = 100;
const DEFAULT_COOKIE_PATH: &str = "/";
const SESSION_COOKIE_EXPIRES: f64 = -1.0;
const HAR_MIME: &str = "application/json";
const HAR_FILENAME: &str = "recording.har";

tab_op!(COOKIES_GET {
    tool: "cookies_get",
    cli: "cookies-get",
    summary: "Get the Tab's cookies, optionally only those that apply to the given URLs",
    positional: &[],
    read_only: true,
    capability: None,
    input: CookiesGetInput,
    output: CookiesOut,
    run: cookies_get,
});
tab_op!(COOKIES_SET {
    tool: "cookies_set",
    cli: "cookies-set",
    summary: "Set cookies in the Tab's browser context",
    positional: &[],
    read_only: false,
    capability: None,
    input: CookiesSetInput,
    output: Tab,
    run: cookies_set,
});
tab_op!(COOKIES_CLEAR {
    tool: "cookies_clear",
    cli: "cookies-clear",
    summary: "Delete every cookie of the Tab's browser context",
    positional: &[],
    read_only: false,
    capability: None,
    input: TabInput,
    output: Tab,
    run: cookies_clear,
});
tab_op!(STORAGE_GET {
    tool: "storage_get",
    cli: "storage-get",
    summary: "Read the page's localStorage or sessionStorage",
    positional: &[],
    read_only: true,
    capability: None,
    input: StorageAreaInput,
    output: StorageOut,
    run: storage_get,
});
tab_op!(STORAGE_SET {
    tool: "storage_set",
    cli: "storage-set",
    summary: "Set items in the page's localStorage or sessionStorage",
    positional: &[],
    read_only: false,
    capability: None,
    input: StorageSetInput,
    output: Tab,
    run: storage_set,
});
tab_op!(STORAGE_CLEAR {
    tool: "storage_clear",
    cli: "storage-clear",
    summary: "Clear the page's localStorage or sessionStorage",
    positional: &[],
    read_only: false,
    capability: None,
    input: StorageAreaInput,
    output: Tab,
    run: storage_clear,
});
tab_op!(CONSOLE_MESSAGES {
    tool: "console_messages",
    cli: "console-messages",
    summary: "Read the console messages the Tab buffered, newest last",
    positional: &[],
    read_only: true,
    capability: None,
    input: ConsoleInput,
    output: MessagesOut,
    run: console_messages,
});
tab_op!(NETWORK_LOG {
    tool: "network_log",
    cli: "network-log",
    summary: "Read the network requests the Tab buffered, newest last",
    positional: &[],
    read_only: true,
    capability: None,
    input: NetworkLogInput,
    output: EntriesOut,
    run: network_log,
});
tab_op!(HAR_START {
    tool: "har_start",
    cli: "har-start",
    summary: "Start recording the Tab's network traffic as a HAR",
    positional: &[],
    read_only: false,
    capability: Some(brr_core::Capability::Har),
    input: TabInput,
    output: Tab,
    run: har_start,
});
tab_op!(HAR_STOP {
    tool: "har_stop",
    cli: "har-stop",
    summary: "Stop the recording and store it as a HAR Artifact",
    positional: &[],
    read_only: false,
    capability: Some(brr_core::Capability::Har),
    input: HarStopInput,
    output: Delivery,
    run: har_stop,
});
tab_op!(EMULATE {
    tool: "emulate",
    cli: "emulate",
    summary: "Change what the Tab emulates: viewport, device, geolocation, timezone, offline mode, color scheme",
    positional: &[],
    read_only: false,
    capability: None,
    input: EmulateInput,
    output: Tab,
    run: emulate,
});
tab_op!(BLOCK_REQUESTS {
    tool: "block_requests",
    cli: "block-requests",
    summary: "Block requests by URL pattern or resource type; both empty lifts the block",
    positional: &[],
    read_only: false,
    capability: Some(brr_core::Capability::RequestInterception),
    input: BlockRequestsInput,
    output: Tab,
    run: block_requests,
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CookiesGetInput {
    tab_id: Option<TabId>,
    urls: Option<Vec<String>>,
}

#[derive(Serialize, JsonSchema)]
struct CookiesOut {
    cookies: Vec<Cookie>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CookiesSetInput {
    tab_id: Option<TabId>,
    cookies: Vec<CookieInput>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StorageAreaInput {
    tab_id: Option<TabId>,
    area: StorageArea,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StorageSetInput {
    tab_id: Option<TabId>,
    area: StorageArea,
    items: BTreeMap<String, String>,
}

#[derive(Serialize, JsonSchema)]
struct StorageOut {
    items: BTreeMap<String, String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ConsoleInput {
    tab_id: Option<TabId>,
    level: Option<String>,
    limit: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
struct MessagesOut {
    messages: Vec<ConsoleMessage>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NetworkLogInput {
    tab_id: Option<TabId>,
    limit: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
struct EntriesOut {
    entries: Vec<NetworkEntry>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HarStopInput {
    tab_id: Option<TabId>,
    #[serde(rename = "return")]
    return_mode: Option<ReturnMode>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmulateInput {
    tab_id: Option<TabId>,
    viewport: Option<Size>,
    device: Option<String>,
    geolocation: Option<Geolocation>,
    timezone: Option<String>,
    offline: Option<bool>,
    color_scheme: Option<ColorScheme>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BlockRequestsInput {
    tab_id: Option<TabId>,
    patterns: Vec<String>,
    types: Vec<ResourceType>,
}

// The newest `limit` items of an oldest-first list.
fn newest<T>(mut items: Vec<T>, limit: u32) -> Vec<T> {
    let keep = usize::try_from(limit).unwrap_or(usize::MAX);
    let skip = items.len().saturating_sub(keep);
    items.drain(..skip);

    items
}

async fn cookies_get(ctx: Ctx, input: CookiesGetInput) -> Result<Value, Error> {
    let urls = input.urls.unwrap_or_default();
    if let Some(bad) = urls.iter().find(|url| reqwest::Url::parse(url).is_err()) {
        return Err(invalid("urls", &format!("{bad:?} is not an absolute URL")));
    }
    let call = tab_call(&ctx, input.tab_id, &COOKIES_GET).await?;

    let cookies = call.page.cookies(&urls).await?;

    output(&CookiesOut { cookies })
}

fn cookie_from_input(input: CookieInput) -> Result<Cookie, Error> {
    let url = input
        .url
        .as_deref()
        .map(|url| {
            reqwest::Url::parse(url)
                .map_err(|_| invalid("cookies", &format!("{url:?} is not an absolute URL")))
        })
        .transpose()?;
    let domain = match (input.domain, url.as_ref().and_then(reqwest::Url::host_str)) {
        (Some(domain), _) => domain,
        (None, Some(host)) => host.to_string(),
        (None, None) => {
            return Err(invalid(
                "cookies",
                &format!(
                    "cookie {:?} needs a domain or a url with a host",
                    input.name
                ),
            ));
        }
    };
    let https = url.as_ref().is_some_and(|url| url.scheme() == "https");

    Ok(Cookie {
        name: input.name,
        value: input.value,
        domain,
        path: input
            .path
            .unwrap_or_else(|| DEFAULT_COOKIE_PATH.to_string()),
        expires: input.expires.unwrap_or(SESSION_COOKIE_EXPIRES),
        http_only: input.http_only.unwrap_or(false),
        secure: input.secure.unwrap_or(https),
        same_site: input.same_site.unwrap_or(SameSite::Lax),
    })
}

async fn cookies_set(ctx: Ctx, input: CookiesSetInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &COOKIES_SET).await?;

    let cookies = input
        .cookies
        .into_iter()
        .map(cookie_from_input)
        .collect::<Result<Vec<_>, _>>()?;
    call.page.set_cookies(&cookies).await?;

    call.tab_out(&ctx).await
}

async fn cookies_clear(ctx: Ctx, input: TabInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &COOKIES_CLEAR).await?;

    call.page.clear_cookies().await?;

    call.tab_out(&ctx).await
}

async fn storage_get(ctx: Ctx, input: StorageAreaInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &STORAGE_GET).await?;

    let items = call.page.storage(input.area).await?;

    output(&StorageOut { items })
}

async fn storage_set(ctx: Ctx, input: StorageSetInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &STORAGE_SET).await?;

    call.page.set_storage(input.area, &input.items).await?;

    call.tab_out(&ctx).await
}

async fn storage_clear(ctx: Ctx, input: StorageAreaInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &STORAGE_CLEAR).await?;

    call.page.clear_storage(input.area).await?;

    call.tab_out(&ctx).await
}

async fn console_messages(ctx: Ctx, input: ConsoleInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &CONSOLE_MESSAGES).await?;

    let mut messages = call.page.console_messages();
    if let Some(level) = &input.level {
        messages.retain(|message| message.level.eq_ignore_ascii_case(level));
    }
    let messages = newest(messages, input.limit.unwrap_or(DEFAULT_LIMIT));

    output(&MessagesOut { messages })
}

async fn network_log(ctx: Ctx, input: NetworkLogInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &NETWORK_LOG).await?;

    let entries = newest(
        call.page.network_log(),
        input.limit.unwrap_or(DEFAULT_LIMIT),
    );

    output(&EntriesOut { entries })
}

async fn har_start(ctx: Ctx, input: TabInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &HAR_START).await?;

    call.page.har_start()?;

    call.tab_out(&ctx).await
}

async fn har_stop(ctx: Ctx, input: HarStopInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &HAR_STOP).await?;

    let har = call.page.har_stop()?;

    // A recording is always stored: it is too valuable to hand back only inline.
    let mode = match input.return_mode.unwrap_or_default() {
        ReturnMode::Inline => ReturnMode::Both,
        ReturnMode::Auto | ReturnMode::Url => ReturnMode::Url,
        ReturnMode::Both => ReturnMode::Both,
    };

    deliver(
        &ctx,
        &call,
        ArtifactKind::Har,
        HAR_FILENAME,
        HAR_MIME,
        har,
        Some(mode),
    )
    .await
}

async fn emulate(ctx: Ctx, input: EmulateInput) -> Result<Value, Error> {
    if let Some(place) = &input.geolocation {
        if !(-MAX_LATITUDE..=MAX_LATITUDE).contains(&place.latitude) {
            return Err(invalid(
                "geolocation.latitude",
                &format!("must be within -{MAX_LATITUDE}..={MAX_LATITUDE}"),
            ));
        }
        if !(-MAX_LONGITUDE..=MAX_LONGITUDE).contains(&place.longitude) {
            return Err(invalid(
                "geolocation.longitude",
                &format!("must be within -{MAX_LONGITUDE}..={MAX_LONGITUDE}"),
            ));
        }
        if place
            .accuracy
            .is_some_and(|accuracy| accuracy < 0.0 || accuracy.is_nan())
        {
            return Err(invalid("geolocation.accuracy", "must be at least 0"));
        }
    }
    if let Some(size) = &input.viewport
        && (size.width == 0 || size.height == 0)
    {
        return Err(invalid("viewport", "width and height must be at least 1"));
    }
    let options = EmulateOptions {
        viewport: input.viewport,
        device: input.device,
        geolocation: input.geolocation,
        timezone: input.timezone,
        offline: input.offline,
        color_scheme: input.color_scheme,
        ..EmulateOptions::default()
    };
    if options == EmulateOptions::default() {
        return Err(invalid("viewport", "pass at least one field to emulate"));
    }
    let call = tab_call(&ctx, input.tab_id, &EMULATE).await?;

    call.page.emulate(&options).await?;

    call.tab_out(&ctx).await
}

async fn block_requests(ctx: Ctx, input: BlockRequestsInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &BLOCK_REQUESTS).await?;

    call.page
        .set_request_rules(RequestRules {
            patterns: input.patterns,
            types: input.types,
        })
        .await?;

    call.tab_out(&ctx).await
}

#[cfg(test)]
mod tests;
