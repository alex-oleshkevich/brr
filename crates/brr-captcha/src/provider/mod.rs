use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use brr_core::{CaptchaType, Error, MICROS_PER_USD, ProxyScheme, Secret};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::time::sleep;

use crate::detect::Detected;

#[cfg(test)]
mod tests;

pub const PROVIDER_NAME: &str = "2captcha";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY_BYTES: usize = 1024 * 1024;
// Faster polling gets the key rate limited (ERROR_TOO_MUCH_REQUESTS)
const POLL_INTERVAL: Duration = Duration::from_secs(5);
const IMAGE_FIRST_POLL: Duration = Duration::from_secs(2);
const RECAPTCHA_V3_MIN_SCORE: f64 = 0.7;
const MICROS_DIGITS: usize = 6;
const MAX_LOGGED_CODE_CHARS: usize = 64;

pub enum Provider {
    TwoCaptcha(TwoCaptcha),
}

pub struct TwoCaptcha {
    http: reqwest::Client,
    base_url: String,
    api_key: Secret,
}

pub struct ProxyTask {
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<Secret>,
}

pub struct Challenge {
    pub detected: Detected,
    pub page_url: String,
    pub image_png: Option<Vec<u8>>,
    pub proxy: Option<ProxyTask>,
}

#[derive(Debug)]
pub enum Solution {
    Token(Secret),
    Geetest(Value),
    Text(Secret),
}

#[derive(Debug)]
pub struct Solved {
    pub solution: Solution,
    pub cost_usd_micros: u64,
}

#[derive(Serialize)]
struct Request<'a> {
    #[serde(rename = "clientKey")]
    client_key: &'a str,
    #[serde(flatten)]
    fields: Value,
}

impl Provider {
    pub fn name(&self) -> &'static str {
        match self {
            Provider::TwoCaptcha(_) => PROVIDER_NAME,
        }
    }

    pub async fn solve(&self, challenge: &Challenge, timeout: Duration) -> Result<Solved, Error> {
        match self {
            Provider::TwoCaptcha(provider) => provider.solve(challenge, timeout).await,
        }
    }

    pub async fn balance(&self) -> Result<u64, Error> {
        match self {
            Provider::TwoCaptcha(provider) => provider.balance().await,
        }
    }
}

impl TwoCaptcha {
    pub fn new(http: reqwest::Client, base_url: String, api_key: Secret) -> TwoCaptcha {
        TwoCaptcha {
            http,
            base_url,
            api_key,
        }
    }

    async fn solve(&self, challenge: &Challenge, timeout: Duration) -> Result<Solved, Error> {
        let task = task_json(challenge)?;
        let first_poll = match challenge.detected.captcha_type {
            CaptchaType::Image => IMAGE_FIRST_POLL,
            _ => POLL_INTERVAL,
        };

        let outcome = tokio::time::timeout(timeout, async {
            let created = self.call("createTask", json!({"task": task})).await?;
            let task_id = created.get("taskId").cloned().ok_or_else(|| {
                tracing::info!(provider = PROVIDER_NAME, "createTask returned no taskId");
                Error::CaptchaFailed
            })?;

            let mut delay = first_poll;
            loop {
                sleep(delay).await;
                delay = POLL_INTERVAL;

                let result = self
                    .call("getTaskResult", json!({"taskId": task_id}))
                    .await?;
                if result["status"] == "ready" {
                    return solved(challenge.detected.captcha_type, &result);
                }
            }
        })
        .await;

        outcome.unwrap_or_else(|_| {
            tracing::info!(provider = PROVIDER_NAME, "captcha solve timed out");
            Err(Error::CaptchaFailed)
        })
    }

    async fn balance(&self) -> Result<u64, Error> {
        let reply = self.call("getBalance", json!({})).await?;

        micros(&reply["balance"]).ok_or_else(|| {
            tracing::info!(
                provider = PROVIDER_NAME,
                "getBalance returned no usable balance"
            );
            Error::CaptchaFailed
        })
    }

    // The key only ever leaves in this JSON body; failures log the provider's error code, never the request
    async fn call(&self, path: &str, fields: Value) -> Result<Value, Error> {
        let body = Request {
            client_key: self.api_key.expose(),
            fields,
        };

        let response = self
            .http
            .post(format!("{}/{path}", self.base_url.trim_end_matches('/')))
            .timeout(REQUEST_TIMEOUT)
            .json(&body)
            .send()
            .await
            .map_err(|source| request_failed(path, &source))?;
        let bytes = read_capped(response, path).await?;

        let reply: Value = serde_json::from_slice(&bytes).map_err(|_| {
            tracing::info!(provider = PROVIDER_NAME, path, "provider reply is not JSON");
            Error::CaptchaFailed
        })?;
        if reply["errorId"].as_i64().unwrap_or(0) != 0 {
            let code: String = reply["errorCode"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(MAX_LOGGED_CODE_CHARS)
                .collect();
            tracing::info!(provider = PROVIDER_NAME, path, error_code = ?code, "provider rejected the request");
            return Err(Error::CaptchaFailed);
        }

        Ok(reply)
    }
}

fn request_failed(path: &str, source: &reqwest::Error) -> Error {
    tracing::info!(
        provider = PROVIDER_NAME,
        path,
        timed_out = source.is_timeout(),
        "provider request failed"
    );
    Error::CaptchaFailed
}

async fn read_capped(mut response: reqwest::Response, path: &str) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| request_failed(path, &source))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            tracing::info!(
                provider = PROVIDER_NAME,
                path,
                "provider reply exceeds the size cap"
            );
            return Err(Error::CaptchaFailed);
        }
        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

fn task_json(challenge: &Challenge) -> Result<Value, Error> {
    let detected = &challenge.detected;
    let captcha_type = detected.captcha_type;
    // 2captcha has no proxy variant of these two
    let proxy = match captcha_type {
        CaptchaType::RecaptchaV3 | CaptchaType::Image => None,
        _ => challenge.proxy.as_ref(),
    };
    let proxyless = if proxy.is_some() { "" } else { "Proxyless" };
    let mut task = Map::new();

    let task_type = match captcha_type {
        CaptchaType::RecaptchaV2 => {
            site_fields(&mut task, challenge);
            task.insert("isInvisible".into(), json!(detected.invisible));
            match (&detected.data_s, detected.enterprise) {
                (Some(data_s), true) => {
                    task.insert("enterprisePayload".into(), json!({"s": data_s}));
                }
                (Some(data_s), false) => {
                    task.insert("recaptchaDataSValue".into(), json!(data_s));
                }
                (None, _) => {}
            }
            let enterprise = if detected.enterprise {
                "Enterprise"
            } else {
                ""
            };
            format!("RecaptchaV2{enterprise}Task{proxyless}")
        }
        CaptchaType::RecaptchaV3 => {
            site_fields(&mut task, challenge);
            task.insert("isEnterprise".into(), json!(detected.enterprise));
            insert_some(&mut task, "pageAction", &detected.action);
            task.insert("minScore".into(), json!(RECAPTCHA_V3_MIN_SCORE));
            "RecaptchaV3TaskProxyless".into()
        }
        CaptchaType::Hcaptcha => {
            site_fields(&mut task, challenge);
            format!("HCaptchaTask{proxyless}")
        }
        CaptchaType::Turnstile => {
            site_fields(&mut task, challenge);
            insert_some(&mut task, "action", &detected.action);
            insert_some(&mut task, "data", &detected.data_s);
            format!("TurnstileTask{proxyless}")
        }
        CaptchaType::Funcaptcha => {
            task.insert("websiteURL".into(), json!(challenge.page_url));
            task.insert("websitePublicKey".into(), json!(detected.sitekey));
            insert_some(
                &mut task,
                "funcaptchaApiJSSubdomain",
                &detected.api_subdomain,
            );
            format!("FunCaptchaTask{proxyless}")
        }
        CaptchaType::Geetest => {
            task.insert("websiteURL".into(), json!(challenge.page_url));
            task.insert("gt".into(), json!(detected.gt));
            insert_some(&mut task, "challenge", &detected.challenge);
            task.insert("version".into(), json!(detected.geetest_version));
            if detected.geetest_version == Some(4) {
                task.insert("initParameters".into(), json!({"captcha_id": detected.gt}));
            }
            format!("GeeTestTask{proxyless}")
        }
        CaptchaType::Image => {
            let Some(png) = &challenge.image_png else {
                tracing::info!(
                    provider = PROVIDER_NAME,
                    "image captcha without image bytes"
                );
                return Err(Error::CaptchaFailed);
            };
            task.insert("body".into(), json!(STANDARD.encode(png)));
            "ImageToTextTask".into()
        }
    };
    task.insert("type".into(), json!(task_type));

    if let Some(proxy) = proxy {
        let proxy_type = match proxy.scheme {
            ProxyScheme::Http | ProxyScheme::Https => "http",
            ProxyScheme::Socks5 => "socks5",
        };
        task.insert("proxyType".into(), json!(proxy_type));
        task.insert("proxyAddress".into(), json!(proxy.host));
        task.insert("proxyPort".into(), json!(proxy.port));
        insert_some(&mut task, "proxyLogin", &proxy.username);
        if let Some(password) = &proxy.password {
            task.insert("proxyPassword".into(), json!(password.expose()));
        }
    }

    Ok(Value::Object(task))
}

fn site_fields(task: &mut Map<String, Value>, challenge: &Challenge) {
    task.insert("websiteURL".into(), json!(challenge.page_url));
    task.insert("websiteKey".into(), json!(challenge.detected.sitekey));
}

fn insert_some(task: &mut Map<String, Value>, key: &str, value: &Option<String>) {
    if let Some(value) = value {
        task.insert(key.into(), json!(value));
    }
}

fn solved(captcha_type: CaptchaType, result: &Value) -> Result<Solved, Error> {
    let solution = &result["solution"];
    let text = |key: &str| solution[key].as_str().map(Secret::new);

    let solution = match captcha_type {
        CaptchaType::RecaptchaV2 | CaptchaType::RecaptchaV3 | CaptchaType::Hcaptcha => {
            text("gRecaptchaResponse").map(Solution::Token)
        }
        CaptchaType::Turnstile | CaptchaType::Funcaptcha => text("token").map(Solution::Token),
        CaptchaType::Geetest => solution
            .is_object()
            .then(|| Solution::Geetest(solution.clone())),
        CaptchaType::Image => text("text").map(Solution::Text),
    };

    let Some(solution) = solution else {
        tracing::info!(
            provider = PROVIDER_NAME,
            "ready task has no usable solution"
        );
        return Err(Error::CaptchaFailed);
    };

    Ok(Solved {
        solution,
        cost_usd_micros: micros(&result["cost"]).unwrap_or(0),
    })
}

// Parses the decimal text, never an f64: 0.00299 must be exactly 2990
fn micros(value: &Value) -> Option<u64> {
    let number;
    let text = match value {
        Value::String(text) => text.as_str(),
        Value::Number(count) => {
            number = count.to_string();
            number.as_str()
        }
        _ => return None,
    };
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if !whole
        .bytes()
        .chain(fraction.bytes())
        .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let fraction: String = fraction
        .chars()
        .chain(std::iter::repeat('0'))
        .take(MICROS_DIGITS)
        .collect();

    whole
        .parse::<u64>()
        .ok()?
        .checked_mul(MICROS_PER_USD)?
        .checked_add(fraction.parse().ok()?)
}
