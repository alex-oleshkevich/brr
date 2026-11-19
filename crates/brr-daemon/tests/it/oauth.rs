use crate::common;

use reqwest::{
    StatusCode, Url,
    header::{CONTENT_TYPE, LOCATION},
    redirect::Policy,
};
use serde_json::{Value, json};

use common::{listen, scratch_app};

const REDIRECT_URI: &str = "http://127.0.0.1:1/callback";
const STATE: &str = "a b&c";
const FORM: &str = "application/x-www-form-urlencoded";

#[tokio::test]
async fn allow_completes_the_authorization_code_flow() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let http = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .unwrap();

    let metadata: Value = http
        .get(format!(
            "http://{addr}/.well-known/oauth-authorization-server"
        ))
        .header("X-Forwarded-Proto", "https")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        metadata["authorization_endpoint"],
        format!("https://{addr}/oauth/authorize")
    );

    let client: Value = http
        .post(format!("http://{addr}/oauth/register"))
        .json(&json!({ "redirect_uris": [REDIRECT_URI] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let client_id = client["client_id"].as_str().unwrap();

    let authorize = format!("http://{addr}/oauth/authorize");
    let query = [
        ("client_id", client_id),
        ("redirect_uri", REDIRECT_URI),
        ("state", STATE),
        ("response_type", "code"),
    ];
    let page = http.get(&authorize).query(&query).send().await.unwrap();
    assert!(page.text().await.unwrap().contains("Allow"));

    let allowed = http.post(&authorize).query(&query).send().await.unwrap();
    assert_eq!(allowed.status(), StatusCode::SEE_OTHER);
    let location = Url::parse(allowed.headers()[LOCATION].to_str().unwrap()).unwrap();
    let pairs: Vec<(String, String)> = location.query_pairs().into_owned().collect();
    assert!(location.as_str().starts_with(REDIRECT_URI));
    assert_eq!(pairs[0].0, "code");
    assert_eq!(pairs[1], ("state".to_string(), STATE.to_string()));

    let tokens: Value = http
        .post(format!("http://{addr}/oauth/token"))
        .header(CONTENT_TYPE, FORM)
        .body(format!("grant_type=authorization_code&code={}", pairs[0].1))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tokens["token_type"], "Bearer");
    assert!(tokens["access_token"].is_string());
}
