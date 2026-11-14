use std::time::Duration;

use brr_core::{Error, OperationSpec, Tab, TabId, WaitUntil, file::Upload};
use brr_engine::page::{ClickOptions, Modifier, MouseButton, Target, UploadFile, WaitCondition};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    invalid, output,
    tool::{optional_target, tab_call, tab_op, timeout},
};
use crate::app::Ctx;

const MAX_CLICK_COUNT: u32 = 3;
const DEFAULT_SCROLL_DY: f64 = 500.0;
const DEFAULT_MIME: &str = "application/octet-stream";
const MIME_BY_EXTENSION: [(&str, &str); 12] = [
    ("txt", "text/plain"),
    ("csv", "text/csv"),
    ("html", "text/html"),
    ("json", "application/json"),
    ("pdf", "application/pdf"),
    ("zip", "application/zip"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
];

tab_op!(GO_BACK {
    tool: "go_back",
    cli: "go-back",
    summary: "Go back one history entry; moved is false when there is none",
    positional: &[],
    read_only: false,
    capability: None,
    input: HistoryInput,
    output: MovedOut,
    run: go_back,
});
tab_op!(GO_FORWARD {
    tool: "go_forward",
    cli: "go-forward",
    summary: "Go forward one history entry; moved is false when there is none",
    positional: &[],
    read_only: false,
    capability: None,
    input: HistoryInput,
    output: MovedOut,
    run: go_forward,
});
tab_op!(RELOAD {
    tool: "reload",
    cli: "reload",
    summary: "Reload a Tab and wait until it loads",
    positional: &[],
    read_only: false,
    capability: None,
    input: HistoryInput,
    output: Tab,
    run: reload,
});
tab_op!(CLICK {
    tool: "click",
    cli: "click",
    summary: "Click an element by ref (from snapshot) or selector",
    positional: &[],
    read_only: false,
    capability: None,
    input: ClickInput,
    output: Tab,
    run: click,
});
tab_op!(DBLCLICK {
    tool: "dblclick",
    cli: "dblclick",
    summary: "Double-click an element by ref or selector",
    positional: &[],
    read_only: false,
    capability: None,
    input: DblclickInput,
    output: Tab,
    run: dblclick,
});
tab_op!(HOVER {
    tool: "hover",
    cli: "hover",
    summary: "Move the pointer over an element by ref or selector",
    positional: &[],
    read_only: false,
    capability: None,
    input: ElementInput,
    output: Tab,
    run: hover,
});
tab_op!(TYPE {
    tool: "type",
    cli: "type",
    summary: "Type text key by key into an element, or into the focused element when no ref or selector is given",
    positional: &["text"],
    read_only: false,
    capability: None,
    input: TypeInput,
    output: Tab,
    run: type_text,
});
tab_op!(FILL {
    tool: "fill",
    cli: "fill",
    summary: "Replace the value of an input by ref or selector",
    positional: &["value"],
    read_only: false,
    capability: None,
    input: FillInput,
    output: Tab,
    run: fill,
});
tab_op!(PRESS_KEY {
    tool: "press_key",
    cli: "press-key",
    summary: "Press a key or a combination such as Control+A, on an element or the focused element",
    positional: &["key"],
    read_only: false,
    capability: None,
    input: PressKeyInput,
    output: Tab,
    run: press_key,
});
tab_op!(SELECT_OPTION {
    tool: "select_option",
    cli: "select-option",
    summary: "Select options of a <select> by value or label",
    positional: &[],
    read_only: false,
    capability: None,
    input: SelectOptionInput,
    output: SelectOut,
    run: select_option,
});
tab_op!(CHECK {
    tool: "check",
    cli: "check",
    summary: "Check a checkbox or radio button by ref or selector",
    positional: &[],
    read_only: false,
    capability: None,
    input: ElementInput,
    output: Tab,
    run: check,
});
tab_op!(UNCHECK {
    tool: "uncheck",
    cli: "uncheck",
    summary: "Uncheck a checkbox by ref or selector",
    positional: &[],
    read_only: false,
    capability: None,
    input: ElementInput,
    output: Tab,
    run: uncheck,
});
tab_op!(DRAG {
    tool: "drag",
    cli: "drag",
    summary: "Drag one element onto another, each by ref or selector",
    positional: &[],
    read_only: false,
    capability: None,
    input: DragInput,
    output: Tab,
    run: drag,
});
tab_op!(UPLOAD_FILE {
    tool: "upload_file",
    cli: "upload-file",
    summary: "Set the files of an <input type=file> by ref or selector",
    positional: &[],
    read_only: false,
    capability: None,
    input: UploadFileInput,
    output: Tab,
    run: upload_file,
});
tab_op!(SCROLL {
    tool: "scroll",
    cli: "scroll",
    summary: "Scroll the page by dx and dy pixels (default dy 500), or an element into view",
    positional: &[],
    read_only: false,
    capability: None,
    input: ScrollInput,
    output: Tab,
    run: scroll,
});
tab_op!(WAIT_FOR {
    tool: "wait_for",
    cli: "wait-for",
    summary: "Wait for exactly one of a visible selector, text, a URL (with * wildcards) or network idle",
    positional: &[],
    read_only: false,
    capability: None,
    input: WaitForInput,
    output: Tab,
    run: wait_for,
});
tab_op!(HANDLE_DIALOG {
    tool: "handle_dialog",
    cli: "handle-dialog",
    summary: "Accept or dismiss the open JavaScript dialog",
    positional: &[],
    read_only: false,
    capability: None,
    input: HandleDialogInput,
    output: Tab,
    run: handle_dialog,
});
tab_op!(EVALUATE {
    tool: "evaluate",
    cli: "evaluate",
    summary: "Evaluate a JavaScript expression in the page and return its JSON value",
    positional: &["expression"],
    read_only: false,
    capability: None,
    input: EvaluateInput,
    output: EvaluateOut,
    run: evaluate,
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HistoryInput {
    tab_id: Option<TabId>,
    wait_until: Option<WaitUntil>,
    timeout: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
struct MovedOut {
    tab: Tab,
    moved: bool,
}

#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ClickInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    timeout: Option<u64>,
    button: Option<Button>,
    modifiers: Option<Vec<Modifier>>,
    click_count: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DblclickInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    timeout: Option<u64>,
    button: Option<Button>,
    modifiers: Option<Vec<Modifier>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ElementInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    timeout: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TypeInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    timeout: Option<u64>,
    text: String,
    delay: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FillInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    timeout: Option<u64>,
    value: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PressKeyInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    timeout: Option<u64>,
    key: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SelectOptionInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    timeout: Option<u64>,
    values: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
struct SelectOut {
    selected: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DragInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    to_ref: Option<String>,
    to_selector: Option<String>,
    timeout: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UploadItem {
    name: String,
    mime: Option<String>,
    data: Upload,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UploadFileInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    timeout: Option<u64>,
    files: Vec<UploadItem>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScrollInput {
    tab_id: Option<TabId>,
    r#ref: Option<String>,
    selector: Option<String>,
    timeout: Option<u64>,
    dx: Option<f64>,
    dy: Option<f64>,
    into_view: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WaitForInput {
    tab_id: Option<TabId>,
    selector: Option<String>,
    text: Option<String>,
    url: Option<String>,
    network_idle: Option<bool>,
    timeout: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HandleDialogInput {
    tab_id: Option<TabId>,
    accept: bool,
    prompt_text: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EvaluateInput {
    tab_id: Option<TabId>,
    expression: String,
    timeout: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
struct EvaluateOut {
    result: Value,
}

impl From<Button> for MouseButton {
    fn from(button: Button) -> MouseButton {
        match button {
            Button::Left => MouseButton::Left,
            Button::Right => MouseButton::Right,
            Button::Middle => MouseButton::Middle,
        }
    }
}

fn click_options(
    button: Option<Button>,
    modifiers: Option<Vec<Modifier>>,
    click_count: u32,
) -> ClickOptions {
    ClickOptions {
        button: button.map_or(MouseButton::Left, MouseButton::from),
        modifiers: modifiers.unwrap_or_default(),
        click_count,
    }
}

fn mime_of(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map(|(_, extension)| extension);

    MIME_BY_EXTENSION
        .iter()
        .find(|(known, _)| extension.is_some_and(|e| e.eq_ignore_ascii_case(known)))
        .map_or(DEFAULT_MIME, |(_, mime)| mime)
}

async fn make_current(ctx: &Ctx, tab_id: &TabId) -> Result<(), Error> {
    ctx.app
        .clients
        .set_current_tab(&ctx.app, &ctx.client.client_id, Some(tab_id))
        .await
}

async fn go_back(ctx: Ctx, input: HistoryInput) -> Result<Value, Error> {
    history(ctx, input, &GO_BACK, false).await
}

async fn go_forward(ctx: Ctx, input: HistoryInput) -> Result<Value, Error> {
    history(ctx, input, &GO_FORWARD, true).await
}

async fn history(
    ctx: Ctx,
    input: HistoryInput,
    spec: &'static OperationSpec,
    forward: bool,
) -> Result<Value, Error> {
    let timeout = timeout(input.timeout)?;
    let wait_until = input.wait_until.unwrap_or(WaitUntil::Load);
    let call = tab_call(&ctx, input.tab_id, spec).await?;

    let moved = if forward {
        call.page.go_forward(wait_until, timeout).await?
    } else {
        call.page.go_back(wait_until, timeout).await?
    };
    make_current(&ctx, &call.tab_id).await?;

    output(&MovedOut {
        tab: call.tab(&ctx).await?,
        moved,
    })
}

async fn reload(ctx: Ctx, input: HistoryInput) -> Result<Value, Error> {
    let timeout = timeout(input.timeout)?;
    let call = tab_call(&ctx, input.tab_id, &RELOAD).await?;

    call.page
        .reload(input.wait_until.unwrap_or(WaitUntil::Load), timeout)
        .await?;
    make_current(&ctx, &call.tab_id).await?;

    call.tab_out(&ctx).await
}

async fn click(ctx: Ctx, input: ClickInput) -> Result<Value, Error> {
    let click_count = input.click_count.unwrap_or(1);
    if !(1..=MAX_CLICK_COUNT).contains(&click_count) {
        return Err(invalid(
            "click_count",
            &format!("must be within 1..={MAX_CLICK_COUNT}"),
        ));
    }
    let target = Target::from_input(input.r#ref, input.selector)?;
    let timeout = timeout(input.timeout)?;
    let call = tab_call(&ctx, input.tab_id, &CLICK).await?;

    let options = click_options(input.button, input.modifiers, click_count);
    call.page.click(&target, &options, timeout).await?;

    call.tab_out(&ctx).await
}

async fn dblclick(ctx: Ctx, input: DblclickInput) -> Result<Value, Error> {
    let target = Target::from_input(input.r#ref, input.selector)?;
    let timeout = timeout(input.timeout)?;
    let call = tab_call(&ctx, input.tab_id, &DBLCLICK).await?;

    let options = click_options(input.button, input.modifiers, 2);
    call.page.click(&target, &options, timeout).await?;

    call.tab_out(&ctx).await
}

async fn hover(ctx: Ctx, input: ElementInput) -> Result<Value, Error> {
    let target = Target::from_input(input.r#ref, input.selector)?;
    let timeout = timeout(input.timeout)?;
    let call = tab_call(&ctx, input.tab_id, &HOVER).await?;

    call.page.hover(&target, timeout).await?;

    call.tab_out(&ctx).await
}

async fn type_text(ctx: Ctx, input: TypeInput) -> Result<Value, Error> {
    let target = optional_target(input.r#ref, input.selector)?;
    let timeout = timeout(input.timeout)?;
    let delay = Duration::from_millis(input.delay.unwrap_or(0));
    // The timeout bounds the typing too, so a long text with a delay cannot run on unchecked.
    let chars = u32::try_from(input.text.chars().count()).unwrap_or(u32::MAX);
    if delay.saturating_mul(chars) > timeout {
        return Err(invalid(
            "delay",
            "delay times the text length exceeds the timeout",
        ));
    }
    let call = tab_call(&ctx, input.tab_id, &TYPE).await?;

    call.page
        .type_text(target.as_ref(), &input.text, delay, timeout)
        .await?;

    call.tab_out(&ctx).await
}

async fn fill(ctx: Ctx, input: FillInput) -> Result<Value, Error> {
    let target = Target::from_input(input.r#ref, input.selector)?;
    let timeout = timeout(input.timeout)?;
    let call = tab_call(&ctx, input.tab_id, &FILL).await?;

    call.page.fill(&target, &input.value, timeout).await?;

    call.tab_out(&ctx).await
}

async fn press_key(ctx: Ctx, input: PressKeyInput) -> Result<Value, Error> {
    let target = optional_target(input.r#ref, input.selector)?;
    let timeout = timeout(input.timeout)?;
    let call = tab_call(&ctx, input.tab_id, &PRESS_KEY).await?;

    call.page
        .press_key(target.as_ref(), &input.key, timeout)
        .await?;

    call.tab_out(&ctx).await
}

async fn select_option(ctx: Ctx, input: SelectOptionInput) -> Result<Value, Error> {
    let target = Target::from_input(input.r#ref, input.selector)?;
    let timeout = timeout(input.timeout)?;
    let call = tab_call(&ctx, input.tab_id, &SELECT_OPTION).await?;

    let selected = call
        .page
        .select_option(&target, &input.values, timeout)
        .await?;

    output(&SelectOut { selected })
}

async fn check(ctx: Ctx, input: ElementInput) -> Result<Value, Error> {
    set_checked(ctx, input, true, &CHECK).await
}

async fn uncheck(ctx: Ctx, input: ElementInput) -> Result<Value, Error> {
    set_checked(ctx, input, false, &UNCHECK).await
}

async fn set_checked(
    ctx: Ctx,
    input: ElementInput,
    checked: bool,
    spec: &'static OperationSpec,
) -> Result<Value, Error> {
    let target = Target::from_input(input.r#ref, input.selector)?;
    let timeout = timeout(input.timeout)?;
    let call = tab_call(&ctx, input.tab_id, spec).await?;

    call.page.set_checked(&target, checked, timeout).await?;

    call.tab_out(&ctx).await
}

async fn drag(ctx: Ctx, input: DragInput) -> Result<Value, Error> {
    let from = Target::from_input(input.r#ref, input.selector)?;
    let to = Target::from_input(input.to_ref, input.to_selector)
        .map_err(|_| invalid("to_ref", "pass exactly one of to_ref or to_selector"))?;
    let timeout = timeout(input.timeout)?;
    let call = tab_call(&ctx, input.tab_id, &DRAG).await?;

    call.page.drag(&from, &to, timeout).await?;

    call.tab_out(&ctx).await
}

async fn upload_file(ctx: Ctx, input: UploadFileInput) -> Result<Value, Error> {
    let target = Target::from_input(input.r#ref, input.selector)?;
    let timeout = timeout(input.timeout)?;
    let files: Vec<UploadFile> = input
        .files
        .into_iter()
        .map(|file| UploadFile {
            mime: file.mime.unwrap_or_else(|| mime_of(&file.name).to_owned()),
            name: file.name,
            data: file.data.0,
        })
        .collect();
    let call = tab_call(&ctx, input.tab_id, &UPLOAD_FILE).await?;

    call.page.upload(&target, &files, timeout).await?;

    call.tab_out(&ctx).await
}

async fn scroll(ctx: Ctx, input: ScrollInput) -> Result<Value, Error> {
    let timeout = timeout(input.timeout)?;
    let into_view = input.into_view.unwrap_or(false);
    let target = optional_target(input.r#ref, input.selector)?;
    match (&target, into_view) {
        (None, true) => return Err(invalid("into_view", "needs a ref or selector")),
        (Some(_), false) => return Err(invalid("ref", "a ref or selector needs into_view")),
        (Some(_), true) if input.dx.is_some() || input.dy.is_some() => {
            return Err(invalid("dy", "dx and dy do not combine with into_view"));
        }
        _ => {}
    }
    let call = tab_call(&ctx, input.tab_id, &SCROLL).await?;

    match target {
        Some(target) => call.page.scroll_into_view(&target, timeout).await?,
        None => {
            let dx = input.dx.unwrap_or(0.0);
            let dy = input.dy.unwrap_or(DEFAULT_SCROLL_DY);

            call.page.scroll_by(dx, dy).await?;
        }
    }

    call.tab_out(&ctx).await
}

async fn wait_for(ctx: Ctx, input: WaitForInput) -> Result<Value, Error> {
    let timeout = timeout(input.timeout)?;
    let conditions: Vec<WaitCondition> = [
        input.selector.map(WaitCondition::Selector),
        input.text.map(WaitCondition::Text),
        input.url.map(WaitCondition::Url),
        input
            .network_idle
            .filter(|idle| *idle)
            .map(|_| WaitCondition::NetworkIdle),
    ]
    .into_iter()
    .flatten()
    .collect();
    let [condition] = conditions.as_slice() else {
        return Err(invalid(
            "selector",
            "pass exactly one of selector, text, url or network_idle",
        ));
    };
    let call = tab_call(&ctx, input.tab_id, &WAIT_FOR).await?;

    call.page.wait_for(condition, timeout).await?;

    call.tab_out(&ctx).await
}

async fn handle_dialog(ctx: Ctx, input: HandleDialogInput) -> Result<Value, Error> {
    let call = tab_call(&ctx, input.tab_id, &HANDLE_DIALOG).await?;

    call.page
        .handle_dialog(input.accept, input.prompt_text.as_deref())
        .await?;

    call.tab_out(&ctx).await
}

async fn evaluate(ctx: Ctx, input: EvaluateInput) -> Result<Value, Error> {
    let timeout = timeout(input.timeout)?;
    let call = tab_call(&ctx, input.tab_id, &EVALUATE).await?;

    let result = call.page.evaluate(&input.expression, timeout).await?;

    output(&EvaluateOut { result })
}
