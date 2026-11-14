use std::collections::BTreeMap;
use std::time::Duration;

use brr_core::{ConflictReason, Error};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{DEFAULT_EVALUATE_TIMEOUT, MAX_TEXT_BYTES, Page, cap, value_of};
use crate::chromium::millis;

const MAX_LINKS: usize = 10_000;
const MAX_LINK_TEXT_CHARS: usize = 500;
const SECRET_ON_PAGE_HELP: &str =
    "evaluate is disabled on a page holding a filled secret; navigate first";
const LOGIN_RUNNING_HELP: &str = "evaluate is disabled while a login runs; retry when it ends";
const URL_EXPRESSION: &str = "location.href";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Link {
    pub href: String,
    pub text: String,
    pub rel: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Metadata {
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub canonical: Option<String>,
    pub lang: Option<String>,
    pub og: BTreeMap<String, String>,
}

impl Page {
    pub async fn url(&self) -> Result<String, Error> {
        let value = self
            .evaluate_internal(URL_EXPRESSION, DEFAULT_EVALUATE_TIMEOUT)
            .await?;

        decode(value)
    }

    // For Clients. A page that holds a secret brr typed refuses: redaction only catches the value
    // as the page shows it, and a script can return it in any other form.
    pub async fn evaluate(&self, expression: &str, timeout: Duration) -> Result<Value, Error> {
        let detail = if self.state.evaluate_locked() {
            Some(LOGIN_RUNNING_HELP)
        } else if self.state.holds_secret() {
            Some(SECRET_ON_PAGE_HELP)
        } else {
            None
        };
        if let Some(detail) = detail {
            return Err(Error::Conflict {
                reason: ConflictReason::SecretOnPage,
                detail: Some(detail.to_owned()),
            });
        }

        self.evaluate_with(expression, timeout, true).await
    }

    // For brr's own scripts that run in the page's world and return nothing worth reading back
    // (a captcha token applied on a login page): they skip the Client guards of `evaluate`.
    pub async fn evaluate_own(&self, expression: &str, timeout: Duration) -> Result<Value, Error> {
        self.evaluate_with(expression, timeout, true).await
    }

    // brr's own reads. On Camoufox they stay in its isolated world, which sees the DOM but which
    // the page's scripts cannot observe.
    pub(crate) async fn evaluate_internal(
        &self,
        expression: &str,
        timeout: Duration,
    ) -> Result<Value, Error> {
        self.evaluate_with(expression, timeout, false).await
    }

    // `in_page`: the page's own world, where its scripts' globals are visible.
    async fn evaluate_with(
        &self,
        expression: &str,
        timeout: Duration,
        in_page: bool,
    ) -> Result<Value, Error> {
        let session_id = self.session.main_session();
        let context_id = self
            .state
            .frame(&self.state.main_frame())
            .and_then(|frame| frame.main_context);
        let evaluated = tokio::time::timeout(timeout, async {
            if in_page {
                self.session
                    .evaluate_in_page(session_id, context_id, expression, timeout)
                    .await
            } else {
                self.session
                    .evaluate(session_id, context_id, expression, true, Some(timeout))
                    .await
            }
        })
        .await
        .map_err(|_| Error::Timeout {
            after_ms: millis(timeout),
        })?;

        self.redacted(evaluated)
    }

    // The main frame's utility world: the page's own scripts cannot reach or tamper with it.
    pub async fn evaluate_utility(&self, expression: &str) -> Result<Value, Error> {
        let frame_id = self.state.main_frame();
        let session_id = self.session.main_session();
        let context_id = self
            .session
            .utility_world(session_id, &frame_id, &self.state)
            .await?;
        let evaluated = tokio::time::timeout(
            DEFAULT_EVALUATE_TIMEOUT,
            self.session
                .evaluate(session_id, Some(context_id), expression, true, None),
        )
        .await
        .map_err(|_| Error::Timeout {
            after_ms: millis(DEFAULT_EVALUATE_TIMEOUT),
        })?;

        self.redacted(evaluated)
    }

    // The value and the page's own exception text, which can quote what it was given.
    fn redacted(&self, evaluated: Result<Value, Error>) -> Result<Value, Error> {
        match evaluated {
            Ok(mut remote) => {
                let mut value = value_of(&mut remote);
                self.state.redact_value(&mut value);

                Ok(value)
            }
            Err(Error::EvaluationFailed { message }) => Err(Error::EvaluationFailed {
                message: self.state.redact_owned(message),
            }),
            Err(error) => Err(error),
        }
    }

    pub async fn html(&self) -> Result<String, Error> {
        let expression = format!(
            "(document.doctype ? new XMLSerializer().serializeToString(document.doctype) : '') + document.documentElement.outerHTML.slice(0, {MAX_TEXT_BYTES})"
        );
        let value = self
            .evaluate_internal(&expression, DEFAULT_EVALUATE_TIMEOUT)
            .await?;

        Ok(cap(decode::<String>(value)?, MAX_TEXT_BYTES))
    }

    pub async fn text(&self) -> Result<String, Error> {
        let expression = format!("(document.body?.innerText ?? '').slice(0, {MAX_TEXT_BYTES})");
        let value = self.evaluate_utility(&expression).await?;

        Ok(cap(decode::<String>(value)?, MAX_TEXT_BYTES))
    }

    pub async fn links(&self) -> Result<Vec<Link>, Error> {
        let expression = format!(
            r"(() => {{
                const seen = new Set();
                const links = [];
                for (const anchor of document.querySelectorAll('a[href]')) {{
                    const href = anchor.href;
                    if (typeof href !== 'string' || seen.has(href)) continue;
                    seen.add(href);
                    links.push({{
                        href,
                        text: (anchor.innerText ?? '').trim().slice(0, {MAX_LINK_TEXT_CHARS}),
                        rel: anchor.getAttribute('rel') ?? '',
                    }});
                    if (links.length >= {MAX_LINKS}) break;
                }}
                return links;
            }})()"
        );
        let value = self.evaluate_utility(&expression).await?;

        decode(value)
    }

    pub async fn metadata(&self) -> Result<Metadata, Error> {
        let expression = r#"(() => {
            const content = (selector) => document.querySelector(selector)?.getAttribute('content') ?? null;
            const og = [...document.querySelectorAll('meta[property^="og:"]')]
                .map((meta) => [meta.getAttribute('property'), meta.getAttribute('content') ?? '']);
            return {
                url: location.href,
                title: document.title,
                description: content('meta[name=description]'),
                canonical: document.querySelector('link[rel=canonical]')?.href ?? null,
                lang: document.documentElement.getAttribute('lang'),
                og: Object.fromEntries(og),
            };
        })()"#;
        let value = self.evaluate_utility(expression).await?;

        decode(value)
    }
}

fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|error| Error::Internal {
        message: format!("unexpected page answer: {error}"),
    })
}
