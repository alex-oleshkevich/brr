use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{Json, Router, extract::State, http::Uri};
use brr_core::{CaptchaType, Error, ProxyScheme, Secret};
use serde_json::{Value, json};
use test_case::test_case;
use tokio::{net::TcpListener, time::Instant};

use super::{
    Challenge, POLL_INTERVAL, Provider, ProxyTask, Solution, TwoCaptcha, micros, task_json,
};
use crate::detect::Detected;

const API_KEY: &str = "key-6f1c2d";
const PAGE_URL: &str = "https://example.com/login";
const SITEKEY: &str = "sitekey-1";
const TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Default)]
struct Fake {
    requests: Mutex<Vec<(String, Value)>>,
    // The last reply of a path repeats forever
    replies: Mutex<HashMap<String, VecDeque<Value>>>,
}

impl Fake {
    fn bodies(&self, path: &str) -> Vec<Value> {
        let requests = self.requests.lock().unwrap();
        requests
            .iter()
            .filter(|(p, _)| p == path)
            .map(|(_, body)| body.clone())
            .collect()
    }
}

async fn handle(State(fake): State<Arc<Fake>>, uri: Uri, Json(body): Json<Value>) -> Json<Value> {
    let path = uri.path().trim_start_matches('/').to_string();
    fake.requests.lock().unwrap().push((path.clone(), body));

    let mut replies = fake.replies.lock().unwrap();
    let queue = replies.entry(path).or_default();
    let reply = if queue.len() > 1 {
        queue.pop_front()
    } else {
        queue.front().cloned()
    };

    Json(reply.unwrap_or_else(|| json!({"errorId": 1, "errorCode": "ERROR_UNEXPECTED"})))
}

async fn provider(replies: &[(&str, Value)]) -> (Provider, Arc<Fake>) {
    let fake = Arc::new(Fake::default());
    for (path, reply) in replies {
        fake.replies
            .lock()
            .unwrap()
            .entry(path.to_string())
            .or_default()
            .push_back(reply.clone());
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(handle).with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });

    let two_captcha = TwoCaptcha::new(reqwest::Client::new(), base_url, Secret::new(API_KEY));
    (Provider::TwoCaptcha(two_captcha), fake)
}

fn detected(captcha_type: CaptchaType) -> Detected {
    Detected {
        captcha_type,
        sitekey: Some(SITEKEY.into()),
        action: None,
        data_s: None,
        enterprise: false,
        invisible: false,
        api_subdomain: None,
        gt: None,
        challenge: None,
        geetest_version: None,
        image_selector: None,
        input_selector: None,
    }
}

fn challenge(detected: Detected, proxy: Option<ProxyTask>) -> Challenge {
    Challenge {
        detected,
        page_url: PAGE_URL.into(),
        image_png: None,
        proxy,
    }
}

fn socks_proxy() -> ProxyTask {
    ProxyTask {
        scheme: ProxyScheme::Socks5,
        host: "1.2.3.4".into(),
        port: 1080,
        username: Some("user".into()),
        password: Some(Secret::new("pass")),
    }
}

fn ready_with_token() -> Value {
    json!({"errorId": 0, "status": "ready", "solution": {"gRecaptchaResponse": "tok"}, "cost": "0.00299"})
}

#[tokio::test(start_paused = true)]
async fn recaptcha_v2_proxyless() {
    let (provider, fake) = provider(&[
        ("createTask", json!({"errorId": 0, "taskId": 7})),
        (
            "getTaskResult",
            json!({"errorId": 0, "status": "processing"}),
        ),
        ("getTaskResult", ready_with_token()),
    ])
    .await;
    let started = Instant::now();

    let solved = provider
        .solve(
            &challenge(detected(CaptchaType::RecaptchaV2), None),
            TIMEOUT,
        )
        .await
        .unwrap();

    assert!(matches!(&solved.solution, Solution::Token(token) if token.expose() == "tok"));
    assert_eq!(solved.cost_usd_micros, 2990);
    assert!(started.elapsed() >= 2 * POLL_INTERVAL);

    let created = &fake.bodies("createTask")[0];
    assert_eq!(created["clientKey"], API_KEY);
    assert_eq!(created["task"]["type"], "RecaptchaV2TaskProxyless");
    assert_eq!(created["task"]["websiteURL"], PAGE_URL);
    assert_eq!(created["task"]["websiteKey"], SITEKEY);
    let polled = fake.bodies("getTaskResult");
    assert_eq!(polled.len(), 2);
    assert_eq!(polled[0]["taskId"], 7);
}

#[tokio::test(start_paused = true)]
async fn proxy_task() {
    let (provider, fake) = provider(&[
        ("createTask", json!({"errorId": 0, "taskId": 7})),
        ("getTaskResult", ready_with_token()),
    ])
    .await;

    provider
        .solve(
            &challenge(detected(CaptchaType::Hcaptcha), Some(socks_proxy())),
            TIMEOUT,
        )
        .await
        .unwrap();

    let task = &fake.bodies("createTask")[0]["task"];
    assert_eq!(task["type"], "HCaptchaTask");
    assert_eq!(task["proxyType"], "socks5");
    assert_eq!(task["proxyAddress"], "1.2.3.4");
    assert_eq!(task["proxyPort"], 1080);
    assert_eq!(task["proxyLogin"], "user");
    assert_eq!(task["proxyPassword"], "pass");
}

#[test_case(
    json!({"errorId": 12, "errorCode": "ERROR_CAPTCHA_UNSOLVABLE"}),
    TIMEOUT;
    "provider_error"
)]
#[test_case(
    json!({"errorId": 0, "status": "processing"}),
    Duration::from_secs(12);
    "never_ready"
)]
#[tokio::test(start_paused = true)]
async fn failures(result: Value, timeout: Duration) {
    let (provider, _) = provider(&[
        ("createTask", json!({"errorId": 0, "taskId": 7})),
        ("getTaskResult", result),
    ])
    .await;

    let error = provider
        .solve(&challenge(detected(CaptchaType::Turnstile), None), timeout)
        .await
        .unwrap_err();

    assert!(matches!(error, Error::CaptchaFailed));
    assert!(!format!("{error} {error:?}").contains(API_KEY));
}

#[tokio::test(start_paused = true)]
async fn balance() {
    let (provider, fake) = provider(&[("getBalance", json!({"errorId": 0, "balance": 3.5}))]).await;

    assert_eq!(provider.balance().await.unwrap(), 3_500_000);
    assert_eq!(fake.bodies("getBalance")[0], json!({"clientKey": API_KEY}));
}

#[tokio::test(start_paused = true)]
async fn balance_names_a_bad_key() {
    let (provider, _) = provider(&[(
        "getBalance",
        json!({"errorId": 1, "errorCode": super::KEY_MISSING}),
    )])
    .await;

    let error = provider.balance().await.unwrap_err();

    let Error::InvalidArgument { field, message } = &error else {
        panic!("expected InvalidArgument, got {error:?}");
    };
    assert_eq!(field, super::API_KEY_FIELD);
    assert!(message.contains(super::KEY_MISSING), "{message}");
    assert!(!format!("{error} {error:?}").contains(API_KEY));
}

#[tokio::test(start_paused = true)]
async fn oversized_reply_fails() {
    let big = "x".repeat(super::MAX_BODY_BYTES);
    let (provider, _) = provider(&[(
        "getBalance",
        json!({"errorId": 0, "balance": 1, "pad": big}),
    )])
    .await;

    assert!(matches!(
        provider.balance().await,
        Err(Error::CaptchaFailed)
    ));
}

#[test_case("0.00299", Some(2990); "fraction")]
#[test_case("3.5", Some(3_500_000); "one_digit")]
#[test_case("12", Some(12_000_000); "whole")]
#[test_case("0.0000019", Some(1); "truncates_below_a_micro")]
#[test_case("1e-7", None; "exponent")]
#[test_case("-1", None; "negative")]
fn decimal_text_becomes_micros(text: &str, expected: Option<u64>) {
    assert_eq!(micros(&json!(text)), expected);
}

#[test]
fn recaptcha_v3_ignores_proxy() {
    let mut v3 = detected(CaptchaType::RecaptchaV3);
    v3.action = Some("login".into());

    let task = task_json(&challenge(v3, Some(socks_proxy()))).unwrap();

    assert_eq!(task["type"], "RecaptchaV3TaskProxyless");
    assert_eq!(task["pageAction"], "login");
    assert_eq!(task["minScore"], 0.7);
    assert_eq!(task["isEnterprise"], false);
    assert!(task.get("proxyType").is_none());
}

#[test]
fn recaptcha_v2_enterprise_with_proxy() {
    let mut v2 = detected(CaptchaType::RecaptchaV2);
    v2.enterprise = true;
    v2.invisible = true;
    v2.data_s = Some("s-value".into());

    let task = task_json(&challenge(v2, Some(socks_proxy()))).unwrap();

    assert_eq!(task["type"], "RecaptchaV2EnterpriseTask");
    assert_eq!(task["isInvisible"], true);
    assert_eq!(task["enterprisePayload"], json!({"s": "s-value"}));
    assert!(task.get("recaptchaDataSValue").is_none());
}

#[test]
fn geetest_v4_task() {
    let mut geetest = detected(CaptchaType::Geetest);
    geetest.gt = Some("captcha-id".into());
    geetest.geetest_version = Some(4);

    let task = task_json(&challenge(geetest, None)).unwrap();

    assert_eq!(task["type"], "GeeTestTaskProxyless");
    assert_eq!(task["version"], 4);
    assert_eq!(task["initParameters"], json!({"captcha_id": "captcha-id"}));
}

#[test]
fn funcaptcha_task() {
    let mut funcaptcha = detected(CaptchaType::Funcaptcha);
    funcaptcha.api_subdomain = Some("client-api.arkoselabs.com".into());

    let task = task_json(&challenge(funcaptcha, None)).unwrap();

    assert_eq!(task["type"], "FunCaptchaTaskProxyless");
    assert_eq!(task["websitePublicKey"], SITEKEY);
    assert_eq!(
        task["funcaptchaApiJSSubdomain"],
        "client-api.arkoselabs.com"
    );
}

#[test]
fn image_task_carries_base64_png() {
    let mut image = challenge(detected(CaptchaType::Image), Some(socks_proxy()));
    image.image_png = Some(vec![0x89, b'P', b'N', b'G']);

    let task = task_json(&image).unwrap();

    assert_eq!(task["type"], "ImageToTextTask");
    assert_eq!(task["body"], "iVBORw==");
    assert!(task.get("proxyType").is_none());
}

#[test]
fn image_without_bytes_fails() {
    let image = challenge(detected(CaptchaType::Image), None);

    assert!(matches!(task_json(&image), Err(Error::CaptchaFailed)));
}
