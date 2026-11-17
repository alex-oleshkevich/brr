#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use brr_core::{
    Display, DisplayEnv, Engine, Error, InstanceId, ProfileSettings, TargetId, WaitUntil,
};
use brr_engine::page::{
    ClickOptions, DialogType, Modifier, MouseButton, Page, Target, UploadFile, WaitCondition,
};
use brr_engine::{Driver, LaunchSpec};
use serde_json::json;
use tempfile::TempDir;

const CHROME_ENV: &str = "BRR_TEST_CHROME";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(5);
const SHORT_TIMEOUT: Duration = Duration::from_millis(200);
// The source of the drag has to become stable, which takes frames a loaded machine delays well
// past SHORT_TIMEOUT; only the missing target should use up this wait.
const MISSING_TARGET_TIMEOUT: Duration = Duration::from_secs(2);
const PROMPT_RETURN: Duration = Duration::from_secs(3);
const EVAL_TIMEOUT: Duration = Duration::from_secs(5);
const TYPE_DELAY: Duration = Duration::from_millis(10);
const SITE_PER_PROCESS: &str = "--site-per-process";
const SLOW_BUTTON_URL: &str = "data:text/html,<button id=b style=display:none onclick=count++>B</button><div style='position:fixed;inset:0'></div><script>var count=0;setTimeout(()=>b.style.display='block',300);setTimeout(()=>document.querySelector('div').remove(),600)</script>";
const FORM_URL: &str = "data:text/html,<input id=i><select id=s><option value=a>A<option value=b>B</select><input id=c type=checkbox onchange=changes++><script>var changes=0</script>";
const OOPIF_URL: &str = "data:text/html,<p>top</p><iframe sandbox=allow-scripts srcdoc=\"<button onclick=&quot;this.textContent='clicked'&quot;>Go</button>\" style='width:300px;height:100px'></iframe>";
const DIALOG_URL: &str = "data:text/html,<script>setTimeout(()=>alert('hi'),100)</script>";
const UPLOAD_URL: &str = "data:text/html,<input id=f type=file>";
const LATE_PAGE_URL: &str = "data:text/html,<body><script>setTimeout(() => document.body.append('late-text'), 300); setTimeout(() => { location.href = 'data:text/html,<p>moved</p>'; }, 1500)</script></body>";
const DRAG_URL: &str = "data:text/html,<div id=a style='width:50px;height:50px;background:red'></div><div id=b style='width:50px;height:50px;background:blue'></div><script>var log = []; a.onmousedown = () => log.push('down'); b.onmouseup = () => log.push('up-b');</script>";
const TYPING_DIALOG_URL: &str = "data:text/html,<input id=i oninput=\"alert('typed')\">";
const MODIFIER_URL: &str = "data:text/html,<button id=b>B</button><input id=i><script>var log = []; b.onclick = (e) => log.push('click:' + e.shiftKey); document.onkeydown = (e) => log.push('key:' + e.key + ':' + e.shiftKey);</script>";
const CONFIRM_URL: &str =
    "data:text/html,<button id=b onclick=\"window.answer = confirm('sure')\">B</button>";
const PAGE_A: &str = "data:text/html,<p>a</p>";
const PAGE_B: &str = "data:text/html,<p>b</p>";

fn chrome() -> Option<PathBuf> {
    let chrome = std::env::var_os(CHROME_ENV).map(PathBuf::from);
    if chrome.is_none() {
        eprintln!("skipped: {CHROME_ENV} is not set");
    }

    chrome
}

fn spec(chrome: PathBuf, dir: &Path, args: Vec<String>) -> LaunchSpec {
    let settings = ProfileSettings {
        args,
        ..ProfileSettings::default()
    };

    LaunchSpec {
        instance_id: InstanceId::new(),
        engine: Engine::Chromium,
        executable: chrome,
        user_data_dir: dir.join("profile"),
        log_file: dir.join("logs").join("browser.log"),
        display: Display::Headless,
        display_env: DisplayEnv::default(),
        settings,
        start_timeout: START_TIMEOUT,
        extensions: Vec::new(),
        proxy_server: None,
        download_dir: dir.join("downloads"),
        console_buffer: 1000,
        network_buffer: 1000,
        camoufox: None,
        sandbox: None,
    }
}

struct Harness {
    driver: Driver,
    target_id: TargetId,
    _dir: TempDir,
}

impl Harness {
    async fn start(chrome: PathBuf, args: Vec<String>) -> Harness {
        let dir = TempDir::new().unwrap();
        let launched = Driver::launch(spec(chrome, dir.path(), args))
            .await
            .unwrap();
        let mut events = launched.events;
        tokio::spawn(async move { while events.recv().await.is_some() {} });

        Harness {
            driver: launched.driver,
            target_id: launched.first_target_id,
            _dir: dir,
        }
    }

    async fn open(&self, url: &str) -> Page {
        self.driver
            .navigate(&self.target_id, url, WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .unwrap();

        self.driver.page(&self.target_id).unwrap()
    }

    async fn stop(self) {
        self.driver.close().await.unwrap();
    }
}

fn selector(selector: &str) -> Target {
    Target::Selector(selector.to_owned())
}

fn left_click() -> ClickOptions {
    ClickOptions {
        button: MouseButton::Left,
        modifiers: Vec::new(),
        click_count: 1,
    }
}

// `button "Go" [ref=f1e2]` -> `f1e2`.
fn ref_of(snapshot: &str, needle: &str) -> String {
    let line = snapshot
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line with {needle} in:\n{snapshot}"));
    let start = line.find("[ref=").unwrap() + "[ref=".len();

    line[start..line[start..].find(']').unwrap() + start].to_owned()
}

async fn eval(page: &Page, expression: &str) -> serde_json::Value {
    page.evaluate(expression, EVAL_TIMEOUT).await.unwrap()
}

#[tokio::test]
async fn click_waits_for_actionability() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let page = harness.open(SLOW_BUTTON_URL).await;

    page.click(&selector("#b"), &left_click(), ACTION_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(eval(&page, "count").await, json!(1));

    let page = harness.open(SLOW_BUTTON_URL).await;
    let result = page
        .click(&selector("#b"), &left_click(), SHORT_TIMEOUT)
        .await;
    assert!(matches!(result, Err(Error::Timeout { .. })), "{result:?}");

    harness.stop().await;
}

#[tokio::test]
async fn form_tools() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let page = harness.open(FORM_URL).await;
    let input = selector("#i");

    page.fill(&input, "hello", ACTION_TIMEOUT).await.unwrap();
    page.type_text(Some(&input), " world", TYPE_DELAY, ACTION_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(eval(&page, "i.value").await, json!("hello world"));

    page.press_key(Some(&input), "Control+A", ACTION_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(
        eval(
            &page,
            "i.selectionStart == 0 && i.selectionEnd == i.value.length"
        )
        .await,
        json!(true)
    );

    let chosen = page
        .select_option(&selector("#s"), &["b".to_owned()], ACTION_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(chosen, ["b"]);
    assert_eq!(eval(&page, "s.value").await, json!("b"));

    let checkbox = selector("#c");
    page.set_checked(&checkbox, true, ACTION_TIMEOUT)
        .await
        .unwrap();
    page.set_checked(&checkbox, true, ACTION_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(eval(&page, "c.checked").await, json!(true));
    assert_eq!(eval(&page, "changes").await, json!(1));

    harness.stop().await;
}

#[tokio::test]
async fn oopif_click() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, vec![SITE_PER_PROCESS.to_owned()]).await;
    let page = harness.open(OOPIF_URL).await;

    let snapshot = page.snapshot().await.unwrap();
    let button = ref_of(&snapshot, "button \"Go\"");
    page.click(&Target::Ref(button), &left_click(), ACTION_TIMEOUT)
        .await
        .unwrap();

    let after = page.snapshot().await.unwrap();
    assert!(after.contains("button \"clicked\""), "{after}");

    harness.stop().await;
}

#[tokio::test]
async fn dialogs() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let page = harness.open(DIALOG_URL).await;
    let mut dialog = page.state().dialog();

    let opened = tokio::time::timeout(ACTION_TIMEOUT, dialog.wait_for(Option::is_some))
        .await
        .unwrap()
        .unwrap()
        .clone()
        .unwrap();
    assert_eq!(opened.dialog_type, DialogType::Alert);
    assert_eq!(opened.message, "hi");

    page.handle_dialog(true, None).await.unwrap();
    tokio::time::timeout(ACTION_TIMEOUT, dialog.wait_for(Option::is_none))
        .await
        .unwrap()
        .unwrap();

    let none = page.handle_dialog(true, None).await;
    assert!(matches!(none, Err(Error::NotFound { .. })), "{none:?}");

    harness.stop().await;
}

#[tokio::test]
async fn upload_and_history() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let page = harness.open(UPLOAD_URL).await;

    let file = UploadFile {
        name: "a.txt".to_owned(),
        mime: "text/plain".to_owned(),
        data: b"abc".to_vec(),
    };
    page.upload(&selector("#f"), &[file], ACTION_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(
        eval(&page, "f.files[0].name + ':' + f.files[0].size").await,
        json!("a.txt:3")
    );

    let page = harness.open(PAGE_A).await;
    harness.open(PAGE_B).await;
    assert!(
        page.go_back(WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .unwrap()
    );
    assert_eq!(page.url().await.unwrap(), PAGE_A);
    assert!(
        page.go_forward(WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .unwrap()
    );
    assert!(
        !page
            .go_forward(WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .unwrap()
    );

    harness.stop().await;
}

#[tokio::test]
async fn wait_for_conditions() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;

    let page = harness.open(SLOW_BUTTON_URL).await;
    page.wait_for(&WaitCondition::Selector("#b".to_owned()), ACTION_TIMEOUT)
        .await
        .unwrap();
    let never = page
        .wait_for(
            &WaitCondition::Selector("#nothing".to_owned()),
            SHORT_TIMEOUT,
        )
        .await;
    assert!(matches!(never, Err(Error::Timeout { .. })), "{never:?}");
    page.wait_for(&WaitCondition::NetworkIdle, ACTION_TIMEOUT)
        .await
        .unwrap();

    // The page appends text, then navigates away: the Url wait rides out the navigation.
    let page = harness.open(LATE_PAGE_URL).await;
    page.wait_for(&WaitCondition::Text("late-text".to_owned()), ACTION_TIMEOUT)
        .await
        .unwrap();
    page.wait_for(
        &WaitCondition::Url("data:text/html,*moved*".to_owned()),
        ACTION_TIMEOUT,
    )
    .await
    .unwrap();

    harness.stop().await;
}

#[tokio::test]
async fn drag_presses_moves_and_releases() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let page = harness.open(DRAG_URL).await;

    page.drag(&selector("#a"), &selector("#b"), ACTION_TIMEOUT)
        .await
        .unwrap();

    assert_eq!(eval(&page, "log").await, json!(["down", "up-b"]));

    harness.stop().await;
}

#[tokio::test]
async fn modifiers_are_released_after_a_click() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let page = harness.open(MODIFIER_URL).await;

    let options = ClickOptions {
        modifiers: vec![Modifier::Shift],
        ..left_click()
    };
    page.click(&selector("#b"), &options, ACTION_TIMEOUT)
        .await
        .unwrap();
    page.type_text(Some(&selector("#i")), "a", Duration::ZERO, ACTION_TIMEOUT)
        .await
        .unwrap();

    let log = eval(&page, "log").await;
    assert!(
        log.as_array().unwrap().contains(&json!("click:true")),
        "{log}"
    );
    assert_eq!(log[log.as_array().unwrap().len() - 1], json!("key:a:false"));

    harness.stop().await;
}

#[tokio::test]
async fn a_click_that_opens_a_dialog_returns_at_once() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let page = harness.open(CONFIRM_URL).await;

    let started = std::time::Instant::now();
    page.click(&selector("#b"), &left_click(), ACTION_TIMEOUT)
        .await
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(page.state().dialog().borrow().is_some());

    page.handle_dialog(true, None).await.unwrap();
    assert_eq!(eval(&page, "window.answer").await, json!(true));

    harness.stop().await;
}

// Pages over http: a history navigation between them is a real one (bfcache, no new load).
fn serve(pages: &'static [(&'static str, &'static str)]) -> String {
    use std::io::{BufRead, BufReader, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            reader.read_line(&mut request).unwrap_or_default();
            let path = request.split(' ').nth(1).unwrap_or("/").to_owned();
            let body = pages
                .iter()
                .find(|(name, _)| *name == path)
                .map_or("", |(_, body)| *body);
            let _ = write!(
                reader.get_mut(),
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });

    base
}

const SITE: &[(&str, &str)] = &[("/a", "<p>a</p><input id=i>"), ("/b", "<p>b</p>")];

#[tokio::test]
async fn missing_selector_is_element_not_found() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let base = serve(SITE);
    let page = harness.open(&format!("{base}/a")).await;

    let started = std::time::Instant::now();
    let result = page
        .click(
            &selector("#nope"),
            &left_click(),
            Duration::from_millis(500),
        )
        .await;

    assert_eq!(
        result,
        Err(Error::ElementNotFound),
        "{:?}",
        started.elapsed()
    );

    harness.stop().await;
}

#[tokio::test]
async fn history_moves_over_http_return() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let base = serve(SITE);
    let page = harness.open(&format!("{base}/a")).await;
    harness.open(&format!("{base}/b")).await;

    for wait_until in [WaitUntil::Load, WaitUntil::Domcontentloaded] {
        assert!(
            page.go_back(wait_until, Duration::from_secs(3))
                .await
                .unwrap()
        );
        assert_eq!(page.url().await.unwrap(), format!("{base}/a"));
        assert!(
            page.go_forward(wait_until, Duration::from_secs(3))
                .await
                .unwrap()
        );
        assert_eq!(page.url().await.unwrap(), format!("{base}/b"));
    }

    harness.stop().await;
}

#[tokio::test]
async fn a_drag_to_a_missing_target_presses_nothing() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let page = harness.open(DRAG_URL).await;

    let result = page
        .drag(
            &selector("#a"),
            &selector("#missing"),
            MISSING_TARGET_TIMEOUT,
        )
        .await;

    assert_eq!(result, Err(Error::ElementNotFound));
    assert_eq!(eval(&page, "log").await, json!([]));

    harness.stop().await;
}

#[tokio::test]
async fn typing_that_opens_a_dialog_returns_at_once() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let page = harness.open(TYPING_DIALOG_URL).await;

    let started = std::time::Instant::now();
    page.type_text(Some(&selector("#i")), "a", Duration::ZERO, ACTION_TIMEOUT)
        .await
        .unwrap();
    assert!(started.elapsed() < PROMPT_RETURN, "{:?}", started.elapsed());
    assert!(page.state().dialog().borrow().is_some());

    page.handle_dialog(true, None).await.unwrap();

    harness.stop().await;
}

#[tokio::test]
async fn fill_that_opens_a_dialog_returns_at_once() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome, Vec::new()).await;
    let page = harness.open(TYPING_DIALOG_URL).await;

    let started = std::time::Instant::now();
    page.fill(&selector("#i"), "abc", ACTION_TIMEOUT)
        .await
        .unwrap();
    assert!(started.elapsed() < PROMPT_RETURN, "{:?}", started.elapsed());
    assert!(page.state().dialog().borrow().is_some());

    page.handle_dialog(true, None).await.unwrap();

    harness.stop().await;
}
