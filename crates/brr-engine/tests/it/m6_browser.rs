#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_macros)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use brr_camoufox::fpgen::model::Model;
use brr_content::Adblock;
use brr_core::{
    ConflictReason, Display, DisplayEnv, Engine, Error, Fingerprint, FingerprintOs, InstanceId,
    ProfileSettings, TargetId, WaitUntil,
};
use brr_engine::page::{ContentOptions, Page, Target};
use brr_engine::{CamoufoxSpec, Driver, LaunchSpec, Launched};
use serde_json::{Value, json};
use tempfile::TempDir;

const CHROME_ENV: &str = "BRR_TEST_CHROME";
const CAMOUFOX_ENV: &str = "BRR_TEST_CAMOUFOX";
const MODEL_ENV: &str = "BRR_TEST_FPGEN_MODEL";
const START_TIMEOUT: Duration = Duration::from_secs(60);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(30);
const ACTION_TIMEOUT: Duration = Duration::from_secs(10);
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const ESCAPED_SECRET: &str = "S3cr{et\"!";
const ESCAPED_SECRET_JSON: &str = "S3cr{et\\\"!";
const REDACTED: &str = "«redacted»";
const REJECTED_COOKIE: &str = "consent=rejected";
const HTML: &str = "Content-Type: text/html\r\n";
const SCRIPT: &str = "Content-Type: text/javascript\r\n";

const LOGIN_PAGE: &str = "<form><input id=u type=text name=email><input id=p type=password>\
    <button type=button id=cancel>Cancel</button><button type=submit id=go>Go</button></form>";
// No ids: the selectors have to be tag chains.
const PLAIN_LOGIN_PAGE: &str = "<div><form><input type=hidden name=csrf><input name=email>\
    <input type=password></form></div>";
const OTP_PAGE: &str =
    "<form><input id=c autocomplete=one-time-code><button id=go>Verify</button></form>";
const USERNAME_ONLY_PAGE: &str = "<form><input id=u type=email><input id=gone type=password style='display:none'>\
    <button type=submit id=go>Next</button></form>";
// The attribute copy makes the typed value visible to html() as well.
const FILL_PAGE: &str = "<input id=i oninput=\"this.setAttribute('value', this.value)\">";
const AD_PAGE: &str = "<script src=/ad.js></script><script src=/ok.js></script>";
// The markup of autoconsent's cookie-notice-checkboxes rule; its reject button sets the cookie.
const CONSENT_PAGE: &str = "<div id=cookie-notice>\
    <button id=cookie-notice-accept class=cookie-notice__accept--custom>Accept</button>\
    <button id=cookie-notice-reject onclick=\"document.cookie='consent=rejected; path=/'\">Reject</button></div>";

fn respond(stream: &mut TcpStream, headers: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn handle(mut stream: TcpStream) {
    let mut request = [0_u8; 4096];
    let read = stream.read(&mut request).unwrap_or(0);
    let request = String::from_utf8_lossy(&request[..read]);
    let path = request.split_whitespace().nth(1).unwrap_or("/");

    match path {
        "/login" => respond(&mut stream, HTML, LOGIN_PAGE),
        "/plain-login" => respond(&mut stream, HTML, PLAIN_LOGIN_PAGE),
        "/otp" => respond(&mut stream, HTML, OTP_PAGE),
        "/username" => respond(&mut stream, HTML, USERNAME_ONLY_PAGE),
        "/fill" => respond(&mut stream, HTML, FILL_PAGE),
        "/ads" => respond(&mut stream, HTML, AD_PAGE),
        "/ad.js" => respond(&mut stream, SCRIPT, "window.adLoaded = true;"),
        "/ok.js" => respond(&mut stream, SCRIPT, "window.okLoaded = true;"),
        "/consent" => respond(&mut stream, HTML, CONSENT_PAGE),
        _ => respond(&mut stream, HTML, "ok"),
    }
}

// Pages on 127.0.0.1: a secure context, with cookies, unlike a data: URL.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || handle(stream));
        }
    });

    address.to_string()
}

fn spec(
    engine: Engine,
    executable: PathBuf,
    dir: &Path,
    camoufox: Option<CamoufoxSpec>,
) -> LaunchSpec {
    LaunchSpec {
        instance_id: InstanceId::new(),
        engine,
        executable,
        user_data_dir: dir.join("profile"),
        log_file: dir.join("logs").join("browser.log"),
        display: Display::Headless,
        display_env: DisplayEnv::default(),
        settings: ProfileSettings {
            fingerprint: camoufox.as_ref().map(|_| Fingerprint {
                os: Some(FingerprintOs::Linux),
                ..Fingerprint::default()
            }),
            ..ProfileSettings::default()
        },
        start_timeout: START_TIMEOUT,
        extensions: Vec::new(),
        proxy_server: None,
        download_dir: dir.join("downloads"),
        console_buffer: 1000,
        network_buffer: 1000,
        camoufox,
        sandbox: None,
    }
}

struct Harness {
    driver: Driver,
    target_id: TargetId,
    _dir: TempDir,
    // 127.0.0.1:<port> of the test server.
    origin: String,
}

impl Harness {
    async fn start(spec: impl FnOnce(&Path) -> LaunchSpec) -> Harness {
        let dir = TempDir::new().unwrap();
        let Launched {
            driver,
            first_target_id,
            ..
        } = Driver::launch(spec(dir.path())).await.unwrap();

        Harness {
            driver,
            target_id: first_target_id,
            _dir: dir,
            origin: serve(),
        }
    }

    async fn chromium() -> Option<Harness> {
        let Some(chrome) = std::env::var_os(CHROME_ENV).map(PathBuf::from) else {
            eprintln!("skipped: {CHROME_ENV} is not set");
            return None;
        };

        Some(Harness::start(|dir| spec(Engine::Chromium, chrome, dir, None)).await)
    }

    async fn camoufox() -> Option<Harness> {
        let (Some(executable), Some(model)) = (
            std::env::var_os(CAMOUFOX_ENV).map(PathBuf::from),
            std::env::var_os(MODEL_ENV).map(PathBuf::from),
        ) else {
            eprintln!("skipped: {CAMOUFOX_ENV} or {MODEL_ENV} is not set");
            return None;
        };
        let fonts_dir = executable.parent()?.join("fonts");
        let camoufox = CamoufoxSpec {
            fonts_dir,
            model: Model::open(&model).unwrap(),
            geo: None,
            seed: Some(7),
            addons: Vec::new(),
        };

        Some(Harness::start(|dir| spec(Engine::Camoufox, executable, dir, Some(camoufox))).await)
    }

    async fn open(&self, path: &str) -> Page {
        self.driver
            .navigate(
                &self.target_id,
                &format!("http://{}{path}", self.origin),
                WaitUntil::Load,
                NAVIGATE_TIMEOUT,
            )
            .await
            .unwrap();

        self.driver.page(&self.target_id).unwrap()
    }

    async fn stop(self) {
        self.driver.close().await.unwrap();
    }
}

async fn eval_utility(page: &Page, expression: &str) -> Value {
    page.evaluate_utility(expression).await.unwrap()
}

async fn eval(page: &Page, expression: &str) -> Value {
    page.evaluate(expression, EVAL_TIMEOUT).await.unwrap()
}

// The id of the element a selector found, "" when none.
async fn id_of(page: &Page, selector: &str) -> Value {
    let expression = format!("document.querySelector({})?.id ?? ''", json!(selector));

    eval(page, &expression).await
}

#[tokio::test]
async fn login_fields() {
    let Some(harness) = Harness::chromium().await else {
        return;
    };

    let page = harness.open("/login").await;
    let fields = page.login_fields().await.unwrap();
    assert_eq!(fields.username.as_deref(), Some("#u"));
    assert_eq!(fields.password.as_deref(), Some("#p"));
    assert_eq!(fields.totp, None);
    assert_eq!(fields.submit.as_deref(), Some("#go"));
    assert!(page.password_visible().await.unwrap());

    let page = harness.open("/plain-login").await;
    let fields = page.login_fields().await.unwrap();
    let username = fields.username.unwrap();
    let password = fields.password.unwrap();
    assert_eq!(
        eval(
            &page,
            &format!("document.querySelector({}).name", json!(username))
        )
        .await,
        json!("email")
    );
    assert_eq!(
        eval(
            &page,
            &format!("document.querySelector({}).type", json!(password))
        )
        .await,
        json!("password")
    );
    for selector in [username, password] {
        page.resolve(&Target::Selector(selector), ACTION_TIMEOUT)
            .await
            .unwrap();
    }

    let page = harness.open("/otp").await;
    let fields = page.login_fields().await.unwrap();
    assert_eq!(fields.totp.as_deref(), Some("#c"));
    assert_eq!(fields.password, None);
    assert_eq!(
        id_of(&page, fields.submit.as_deref().unwrap()).await,
        json!("go")
    );

    let page = harness.open("/username").await;
    let fields = page.login_fields().await.unwrap();
    assert_eq!(fields.username.as_deref(), Some("#u"));
    assert_eq!(fields.password, None);
    assert!(!page.password_visible().await.unwrap());

    harness.stop().await;
}

#[tokio::test]
async fn masking() {
    let Some(harness) = Harness::chromium().await else {
        return;
    };
    let page = harness.open("/fill").await;

    // Quoted and escaped in a snapshot: the raw value is not what the page shows.
    let input = Target::Selector("#i".to_owned());
    page.mask(&input, ACTION_TIMEOUT).await.unwrap();
    page.fill(&input, ESCAPED_SECRET, ACTION_TIMEOUT)
        .await
        .unwrap();
    page.state().add_secret(ESCAPED_SECRET);

    assert_eq!(
        eval_utility(
            &page,
            "getComputedStyle(document.querySelector('input')).webkitTextSecurity"
        )
        .await,
        json!("disc")
    );
    let snapshot = page.snapshot().await.unwrap();
    let html = page.html().await.unwrap();
    let value = eval_utility(&page, "document.querySelector('input').value").await;
    for text in [&snapshot, &html, &value.as_str().unwrap().to_owned()] {
        assert!(!text.contains(ESCAPED_SECRET), "{text}");
        assert!(!text.contains(ESCAPED_SECRET_JSON), "{text}");
        assert!(text.contains(REDACTED), "{text}");
    }

    let error = page
        .evaluate_utility("JSON.parse(document.querySelector('input').value)")
        .await
        .unwrap_err();
    assert!(!error.to_string().contains(ESCAPED_SECRET), "{error}");

    let blocked = page.evaluate("1", EVAL_TIMEOUT).await;
    assert!(matches!(
        blocked,
        Err(Error::Conflict {
            reason: ConflictReason::SecretOnPage,
            ..
        })
    ));
    let page = harness.open("/fill").await;
    assert_eq!(eval(&page, "1 + 1").await, json!(2));

    harness.stop().await;
}

#[tokio::test]
async fn adblock_blocks() {
    let Some(harness) = Harness::chromium().await else {
        return;
    };
    let adblock = Adblock::start();
    adblock.replace(vec!["/ad.js$script".to_owned()]);
    let page = harness.driver.page(&harness.target_id).unwrap();

    page.set_content(ContentOptions {
        adblock: Some(adblock),
        dismiss_cookies: false,
    })
    .await
    .unwrap();
    let page = harness.open("/ads").await;
    assert_eq!(
        eval(&page, "typeof window.adLoaded").await,
        json!("undefined")
    );
    assert_eq!(eval(&page, "window.okLoaded").await, json!(true));

    page.set_content(ContentOptions::default()).await.unwrap();
    let page = harness.open("/ads").await;
    assert_eq!(eval(&page, "window.adLoaded").await, json!(true));
    assert_eq!(eval(&page, "window.okLoaded").await, json!(true));

    harness.stop().await;
}

async fn rejects_cookies(harness: Harness) {
    let page = harness.driver.page(&harness.target_id).unwrap();
    page.set_content(ContentOptions {
        adblock: None,
        dismiss_cookies: true,
    })
    .await
    .unwrap();
    let page = harness.open("/consent").await;

    let found = tokio::time::timeout(POLL_TIMEOUT, async {
        loop {
            let cookie = eval(&page, "document.cookie").await;
            if cookie.as_str().is_some_and(|c| c.contains(REJECTED_COOKIE)) {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await;
    assert!(found.is_ok(), "autoconsent did not reject the notice");

    harness.stop().await;
}

#[tokio::test]
async fn autoconsent_rejects() {
    if let Some(harness) = Harness::chromium().await {
        rejects_cookies(harness).await;
    }
}

#[tokio::test]
async fn autoconsent_camoufox() {
    if let Some(harness) = Harness::camoufox().await {
        rejects_cookies(harness).await;
    }
}

#[tokio::test]
async fn login_guards() {
    let Some(harness) = Harness::chromium().await else {
        return;
    };
    let page = harness.open("/login").await;

    let lock = page.lock_evaluate();
    let before = page.navigation_count();
    harness.open("/otp").await;
    tokio::time::timeout(POLL_TIMEOUT, page.wait_navigated(before))
        .await
        .unwrap();

    // The lock outlives the navigation; only dropping it lifts the refusal.
    let refused = page.evaluate("1", EVAL_TIMEOUT).await;
    assert!(matches!(
        refused,
        Err(Error::Conflict {
            reason: ConflictReason::SecretOnPage,
            ..
        })
    ));
    drop(lock);
    assert_eq!(eval(&page, "1 + 1").await, json!(2));

    harness.stop().await;
}
