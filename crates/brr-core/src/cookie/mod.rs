use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::Error;

#[cfg(test)]
mod tests;

const NETSCAPE_HEADER: &str = "# Netscape HTTP Cookie File";
const HTTP_ONLY_PREFIX: &str = "#HttpOnly_";
const NETSCAPE_FIELDS: usize = 7;
const SESSION_EXPIRES: f64 = -1.0;
const DATA_FIELD: &str = "data";

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: f64,
    #[serde(rename = "httpOnly")]
    pub http_only: bool,
    pub secure: bool,
    #[serde(rename = "sameSite")]
    pub same_site: SameSite,
}

/// A cookie to set: name, value and a `domain` or `url` are required, the rest default.
#[derive(Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CookieInput {
    pub name: String,
    pub value: String,
    /// Defaults to the host of `url`.
    pub domain: Option<String>,
    /// Supplies the domain, and the secure flag when it is https and `secure` is not given.
    pub url: Option<String>,
    /// Defaults to "/".
    pub path: Option<String>,
    /// Unix seconds; defaults to a session cookie.
    pub expires: Option<f64>,
    #[serde(rename = "httpOnly")]
    pub http_only: Option<bool>,
    pub secure: Option<bool>,
    /// Defaults to Lax.
    #[serde(rename = "sameSite")]
    pub same_site: Option<SameSite>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JsonCookies {
    StorageState { cookies: Vec<Cookie> },
    Bare(Vec<Cookie>),
}

pub fn to_storage_state(cookies: &[Cookie]) -> String {
    format!(
        "{:#}",
        json!({ "cookies": cookies, "origins": Vec::<Value>::new() })
    )
}

pub fn from_json(text: &str) -> Result<Vec<Cookie>, Error> {
    match serde_json::from_str(text) {
        Ok(JsonCookies::StorageState { cookies } | JsonCookies::Bare(cookies)) => Ok(cookies),
        Err(_) => Err(invalid(
            "expected a Playwright storageState {\"cookies\": [...]} or an array of cookies"
                .to_string(),
        )),
    }
}

pub fn to_netscape(cookies: &[Cookie]) -> String {
    let mut out = format!("{NETSCAPE_HEADER}\n");

    for cookie in cookies {
        let prefix = if cookie.http_only {
            HTTP_ONLY_PREFIX
        } else {
            ""
        };
        let expires = if cookie.expires < 0.0 {
            0
        } else {
            cookie.expires as i64
        };

        out.push_str(&format!(
            "{prefix}{}\t{}\t{}\t{}\t{expires}\t{}\t{}\n",
            cookie.domain,
            flag(cookie.domain.starts_with('.')),
            cookie.path,
            flag(cookie.secure),
            cookie.name,
            cookie.value,
        ));
    }

    out
}

pub fn from_netscape(text: &str) -> Result<Vec<Cookie>, Error> {
    let mut cookies = Vec::new();

    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let (line, http_only) = match line.strip_prefix(HTTP_ONLY_PREFIX) {
            Some(rest) => (rest, true),
            None => (line, false),
        };

        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        let [domain, _, path, secure, expires, name, value] = fields[..] else {
            return Err(invalid(format!(
                "line {number}: expected {NETSCAPE_FIELDS} tab-separated fields"
            )));
        };
        let expires = expires
            .parse::<i64>()
            .map_err(|_| invalid(format!("line {number}: expiry is not a number")))?;

        cookies.push(Cookie {
            name: name.to_string(),
            value: value.to_string(),
            domain: domain.to_string(),
            path: path.to_string(),
            expires: if expires == 0 {
                SESSION_EXPIRES
            } else {
                expires as f64
            },
            http_only,
            secure: secure == "TRUE",
            same_site: SameSite::Lax,
        });
    }

    Ok(cookies)
}

fn flag(on: bool) -> &'static str {
    if on { "TRUE" } else { "FALSE" }
}

fn invalid(message: String) -> Error {
    Error::InvalidArgument {
        field: DATA_FIELD.to_string(),
        message,
    }
}
