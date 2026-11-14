use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{Json, Router, extract::State, http::Uri};
use brr_captcha::{
    detect::Detected,
    provider::{Challenge, Provider, TwoCaptcha},
};
use brr_core::{CaptchaType, ProxyScheme, Secret};
use brr_netproxy::Upstream;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::proxy_task;

const PROXY_USERNAME: &str = "user";
const PROXY_PASSWORD: &str = "pass";

type Requests = Arc<Mutex<Vec<(String, Value)>>>;

async fn handle(
    State(requests): State<Requests>,
    uri: Uri,
    Json(body): Json<Value>,
) -> Json<Value> {
    let path = uri.path().trim_start_matches('/').to_string();
    requests.lock().unwrap().push((path.clone(), body));

    Json(match path.as_str() {
        "createTask" => json!({"errorId": 0, "taskId": 7}),
        _ => json!({
            "errorId": 0,
            "status": "ready",
            "cost": "0.00299",
            "solution": {"gRecaptchaResponse": "tok"},
        }),
    })
}

#[tokio::test(start_paused = true)]
async fn upstream_proxy_passed() {
    let requests = Requests::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(handle).with_state(requests.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });

    let upstream = Upstream {
        scheme: ProxyScheme::Http,
        host: "10.0.0.1".to_string(),
        port: 3128,
        username: Some(PROXY_USERNAME.to_string()),
        password: Some(Secret::new(PROXY_PASSWORD)),
    };
    let challenge = Challenge {
        detected: Detected {
            captcha_type: CaptchaType::RecaptchaV2,
            sitekey: Some("K2".to_string()),
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
        },
        page_url: "https://example.com/".to_string(),
        image_png: None,
        proxy: proxy_task(Some(&upstream)),
    };
    let provider = Provider::TwoCaptcha(TwoCaptcha::new(
        reqwest::Client::new(),
        base_url,
        Secret::new("key"),
    ));

    provider
        .solve(&challenge, Duration::from_secs(60))
        .await
        .unwrap();

    let requests = requests.lock().unwrap();
    let task = &requests
        .iter()
        .find(|(path, _)| path == "createTask")
        .unwrap()
        .1["task"];
    assert_eq!(task["type"], "RecaptchaV2Task");
    assert_eq!(task["proxyAddress"], "10.0.0.1");
    assert_eq!(task["proxyPort"], 3128);
    assert_eq!(task["proxyLogin"], PROXY_USERNAME);
    assert_eq!(task["proxyPassword"], PROXY_PASSWORD);
}
