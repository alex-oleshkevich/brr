use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;

use crate::{Credentials, Error};

const CLOUD: [(&str, &str, &str); 2] = [
    (
        "vault.bitwarden.com",
        "https://identity.bitwarden.com",
        "https://api.bitwarden.com",
    ),
    (
        "vault.bitwarden.eu",
        "https://identity.bitwarden.eu",
        "https://api.bitwarden.eu",
    ),
];
const LOCAL_HOSTS: [&str; 2] = ["localhost", "127.0.0.1"];
const HTTPS: &str = "https";
const HTTP: &str = "http";
const TOKEN_PATH: &str = "/connect/token";
const SYNC_PATH: &str = "/sync";
const SYNC_QUERY: [(&str, &str); 1] = [("excludeDomains", "true")];
const GRANT_TYPE: &str = "client_credentials";
const SCOPE: &str = "api";
const DEVICE_TYPE_SDK: &str = "21";
const DEVICE_NAME: &str = "brr";
const MESSAGE_CAP: usize = 200;
const DEFAULT_AUTH_MESSAGE: &str = "the identity server refused the login";

#[derive(Debug, PartialEq)]
pub(crate) struct Endpoints {
    pub(crate) identity: String,
    pub(crate) api: String,
}

#[derive(Deserialize)]
struct IdentityError {
    error: Option<String>,
    error_description: Option<String>,
    message: Option<String>,
}

pub(crate) fn endpoints(server_url: &str) -> Result<Endpoints, Error> {
    let url = Url::parse(server_url.trim())
        .map_err(|e| Error::InvalidServer(format!("the server URL is not a valid URL ({e})")))?;
    let host = url.host_str().unwrap_or_default();
    let secure = url.scheme() == HTTPS || (url.scheme() == HTTP && LOCAL_HOSTS.contains(&host));
    if host.is_empty() || !secure {
        return Err(Error::InvalidServer(
            "the server URL must start with https:// (http:// only for localhost)".into(),
        ));
    }

    if let Some((_, identity, api)) = CLOUD.iter().find(|(vault, _, _)| *vault == host) {
        return Ok(Endpoints {
            identity: (*identity).into(),
            api: (*api).into(),
        });
    }

    let base = url.as_str().trim_end_matches('/');
    Ok(Endpoints {
        identity: format!("{base}/identity"),
        api: format!("{base}/api"),
    })
}

pub(crate) async fn token(
    http: &Client,
    identity: &str,
    device_id: &str,
    creds: &Credentials,
) -> Result<Vec<u8>, Error> {
    let form = [
        ("grant_type", GRANT_TYPE),
        ("scope", SCOPE),
        ("client_id", creds.client_id.as_str()),
        ("client_secret", creds.client_secret.expose()),
        ("deviceType", DEVICE_TYPE_SDK),
        ("deviceIdentifier", device_id),
        ("deviceName", DEVICE_NAME),
    ];
    let response = http
        .post(format!("{identity}{TOKEN_PATH}"))
        .form(&form)
        .send()
        .await
        .map_err(network)?;
    let status = response.status();
    let body = response.bytes().await.map_err(network)?;

    if status == StatusCode::BAD_REQUEST || status == StatusCode::UNAUTHORIZED {
        return Err(Error::Auth(auth_message(&body)));
    }
    if !status.is_success() {
        return Err(Error::Network(format!(
            "the identity server answered HTTP {status}"
        )));
    }
    Ok(body.into())
}

pub(crate) async fn sync(http: &Client, api: &str, access_token: &str) -> Result<Vec<u8>, Error> {
    let response = http
        .get(format!("{api}{SYNC_PATH}"))
        .query(&SYNC_QUERY)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Network(format!(
            "the sync endpoint answered HTTP {status}"
        )));
    }
    Ok(response.bytes().await.map_err(network)?.into())
}

pub(crate) fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MESSAGE_CAP)
        .collect()
}

// Bitwarden fills error_description; Vaultwarden leaves it empty and puts the reason in message.
fn auth_message(body: &[u8]) -> String {
    serde_json::from_slice::<IdentityError>(body)
        .ok()
        .and_then(|e| {
            [e.error_description, e.message, e.error]
                .into_iter()
                .flatten()
                .find(|message| !message.trim().is_empty())
        })
        .map_or_else(|| DEFAULT_AUTH_MESSAGE.into(), |message| clean(&message))
}

// reqwest's Display stops at "error sending request"; the cause (refused, TLS, DNS) is in the chain.
fn network(error: reqwest::Error) -> Error {
    let error = error.without_url();
    let mut message = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    Error::Network(clean(&message))
}

#[cfg(test)]
mod tests;
