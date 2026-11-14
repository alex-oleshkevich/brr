use std::thread;
use std::time::Duration;

use adblock::Engine;
use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use brr_core::Error;
use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};

#[cfg(test)]
mod tests;

const ADBLOCK_QUEUE: usize = 1024;
pub const MAX_LIST_BYTES: usize = 32 * 1024 * 1024;
const LIST_TIMEOUT: Duration = Duration::from_secs(60);
const SELECTORS_PER_RULE: usize = 1000;

enum Job {
    Replace(Vec<String>),
    Check {
        url: String,
        source_url: String,
        resource_type: &'static str,
        reply: oneshot::Sender<bool>,
    },
    Cosmetic {
        url: String,
        classes: Vec<String>,
        ids: Vec<String>,
        reply: oneshot::Sender<Option<String>>,
    },
}

#[derive(Clone)]
pub struct Adblock {
    jobs: mpsc::Sender<Job>,
}

impl Adblock {
    pub fn start() -> Adblock {
        let (jobs, mut queue) = mpsc::channel(ADBLOCK_QUEUE);

        // The Engine is !Send, so it lives and dies on this thread; the thread ends when the last Adblock drops.
        let spawned = thread::Builder::new()
            .name("brr-adblock".into())
            .spawn(move || {
                let mut engine = Engine::default();
                while let Some(job) = queue.blocking_recv() {
                    match job {
                        Job::Replace(lists) => engine = build(lists),
                        Job::Check {
                            url,
                            source_url,
                            resource_type,
                            reply,
                        } => {
                            let _ = reply.send(check(&engine, &url, &source_url, resource_type));
                        }
                        Job::Cosmetic {
                            url,
                            classes,
                            ids,
                            reply,
                        } => {
                            let _ = reply.send(cosmetic(&engine, &url, &classes, &ids));
                        }
                    }
                }
            });
        // A failed spawn drops the receiver, so every call degrades to "block nothing".
        if let Err(e) = spawned {
            tracing::error!("adblock thread failed to start: {e}");
        }

        Adblock { jobs }
    }

    pub fn replace(&self, lists: Vec<String>) {
        // A full queue means the thread is far behind; the next daily refresh replaces the lists again.
        if self.jobs.try_send(Job::Replace(lists)).is_err() {
            tracing::warn!("adblock list replace dropped: queue full or engine thread gone");
        }
    }

    pub async fn should_block(&self, url: &str, source_url: &str, resource_type: &str) -> bool {
        let (reply, answer) = oneshot::channel();
        let job = Job::Check {
            url: url.to_owned(),
            source_url: source_url.to_owned(),
            resource_type: request_type(resource_type),
            reply,
        };
        if self.jobs.send(job).await.is_err() {
            return false;
        }

        answer.await.unwrap_or(false)
    }

    // `classes` and `ids` are the page's own: the lists' generic `##.class` and `###id` rules
    // apply only to names that occur on the page.
    pub async fn cosmetic_css(
        &self,
        url: &str,
        classes: Vec<String>,
        ids: Vec<String>,
    ) -> Option<String> {
        let (reply, answer) = oneshot::channel();
        let job = Job::Cosmetic {
            url: url.to_owned(),
            classes,
            ids,
            reply,
        };
        self.jobs.send(job).await.ok()?;

        answer.await.ok().flatten()
    }
}

pub async fn fetch_list(http: &reqwest::Client, url: &str) -> Result<String, Error> {
    let response = http
        .get(url)
        .timeout(LIST_TIMEOUT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| internal(format!("adblock list {url}: {e}")))?;

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| internal(format!("adblock list {url}: {e}")))?;
        if body.len() + chunk.len() > MAX_LIST_BYTES {
            return Err(internal(format!(
                "adblock list {url} exceeds {} MiB",
                MAX_LIST_BYTES >> 20
            )));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8(body)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
}

fn build(lists: Vec<String>) -> Engine {
    let mut set = FilterSet::new(false);
    for list in lists {
        set.add_filter_list(list, ParseOptions::default());
    }

    Engine::new_with_filter_set(set)
}

fn check(engine: &Engine, url: &str, source_url: &str, request_type: &str) -> bool {
    Request::new(url, source_url, request_type, "GET")
        .map(|request| engine.check_network_request(&request).should_block())
        .unwrap_or(false)
}

fn cosmetic(engine: &Engine, url: &str, classes: &[String], ids: &[String]) -> Option<String> {
    let resources = engine.url_cosmetic_resources(url);
    let generic = if resources.generichide {
        Vec::new()
    } else {
        engine.hidden_class_id_selectors(classes, ids, &resources.exceptions)
    };
    let selectors: Vec<String> = resources
        .hide_selectors
        .into_iter()
        .chain(generic)
        .collect();
    if selectors.is_empty() {
        return None;
    }

    let rules: Vec<String> = selectors
        .chunks(SELECTORS_PER_RULE)
        .map(|chunk| format!("{}{{display:none!important}}", chunk.join(",")))
        .collect();

    Some(rules.join("\n"))
}

fn request_type(resource_type: &str) -> &'static str {
    match resource_type {
        "Document" => "sub_frame",
        "Stylesheet" => "stylesheet",
        "Image" => "image",
        "Media" => "media",
        "Font" => "font",
        "Script" => "script",
        "XHR" => "xmlhttprequest",
        "Fetch" => "fetch",
        "WebSocket" => "websocket",
        "Ping" => "ping",
        _ => "other",
    }
}

fn internal(message: String) -> Error {
    Error::Internal { message }
}
