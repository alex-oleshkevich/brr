use std::time::Duration;

use brr_cdp::{Cdp, SessionId};
use brr_content::{AUTOCONSENT_JS, Adblock};
use brr_core::Error;
use serde::Deserialize;
use serde_json::json;
use tracing::debug;

use super::{Page, PageState, Session, UTILITY_WORLD};
use crate::chromium::fetch::{PausedRequest, Verdict};
use crate::{Driver, Live, camoufox};

#[cfg(test)]
mod tests;

pub const ADBLOCK_VERDICT_TIMEOUT: Duration = Duration::from_millis(500);

const DOCUMENT: &str = "Document";
const COSMETIC_SCRIPT: &str = "(() => {
    const style = document.createElement('style');
    style.textContent = CSS;
    document.documentElement.appendChild(style);
})()";

// The class and id names the page uses: the lists' generic hiding rules are looked up by them.
const PAGE_NAMES_SCRIPT: &str = "(() => {
    const classes = new Set();
    const ids = new Set();
    for (const element of document.querySelectorAll('[class], [id]')) {
        if (element.id) ids.add(element.id);
        for (const name of element.classList) classes.add(name);
        if (classes.size + ids.size >= MAX_NAMES) break;
    }
    return { classes: [...classes], ids: [...ids] };
})()";
const MAX_PAGE_NAMES: usize = 20_000;

#[derive(Default, Deserialize)]
struct PageNames {
    classes: Vec<String>,
    ids: Vec<String>,
}

// What a Tab does to the pages it loads. Without options of its own a Tab follows the Driver's.
#[derive(Clone, Default)]
pub struct ContentOptions {
    pub adblock: Option<Adblock>,
    pub dismiss_cookies: bool,
}

// A request the adblock thread does not answer in time goes through: a stuck list must not
// stall the page.
async fn adblock_verdict(adblock: &Adblock, request: &PausedRequest) -> Verdict {
    let source_url = request.source_url.as_deref().unwrap_or(&request.url);
    let blocked = tokio::time::timeout(
        ADBLOCK_VERDICT_TIMEOUT,
        adblock.should_block(&request.url, source_url, &request.resource_type),
    )
    .await;

    if blocked == Ok(true) {
        Verdict::Fail
    } else {
        Verdict::Continue
    }
}

// The Tab's request rules first, then its adblock. The page's own document always loads.
pub(crate) async fn verdict(state: &PageState, request: &PausedRequest) -> Verdict {
    if state.verdict(request) == Verdict::Fail {
        return Verdict::Fail;
    }

    let main_document = request.resource_type == DOCUMENT
        && !request.frame_id.is_empty()
        && state.is_main(&request.frame_id);
    match state.adblock() {
        Some(adblock) if !main_document => adblock_verdict(&adblock, request).await,
        _ => Verdict::Continue,
    }
}

// Chromium: once per session, never per navigation; the bundle is ~800 KB.
pub(crate) async fn register_consent(
    cdp: &Cdp,
    session_id: &SessionId,
    state: &PageState,
) -> Result<(), Error> {
    if state.consent_registered(session_id) {
        return Ok(());
    }

    let params = json!({
        "source": AUTOCONSENT_JS,
        "worldName": UTILITY_WORLD,
        "runImmediately": true,
    });
    let result = cdp
        .call(
            Some(session_id),
            "Page.addScriptToEvaluateOnNewDocument",
            params,
        )
        .await?;
    let identifier = result["identifier"]
        .as_str()
        .ok_or_else(|| Error::Internal {
            message: "Page.addScriptToEvaluateOnNewDocument: no identifier".to_owned(),
        })?;
    state.add_consent_script(session_id, identifier.to_owned());

    Ok(())
}

impl Page {
    pub async fn set_content(&self, options: ContentOptions) -> Result<(), Error> {
        self.state.set_content(options);

        self.apply_content().await
    }

    async fn apply_content(&self) -> Result<(), Error> {
        self.apply_interception().await?;

        let dismiss = self.state.dismiss_cookies();
        match &self.session {
            Session::Camoufox(session) => camoufox::init_session(session, dismiss).await,
            Session::Chromium(session) if dismiss => {
                for session_id in self.state.frame_sessions() {
                    register_consent(&session.cdp, &session_id, &self.state).await?;
                }

                Ok(())
            }
            Session::Chromium(session) => {
                for (session_id, identifier) in self.state.take_consent_scripts() {
                    let params = json!({ "identifier": identifier });
                    let removed = session
                        .cdp
                        .call(
                            Some(&session_id),
                            "Page.removeScriptToEvaluateOnNewDocument",
                            params,
                        )
                        .await;
                    if let Err(error) = removed {
                        debug!(%error, "autoconsent script not removed from a session");
                    }
                }

                Ok(())
            }
        }
    }

    // The main frame's cosmetic rules, as a stylesheet in the page. Iframes are not styled.
    pub(crate) fn inject_cosmetic(self) {
        let Some(adblock) = self.state.adblock() else {
            return;
        };

        tokio::spawn(async move {
            let url = self
                .state
                .frame(&self.state.main_frame())
                .map(|frame| frame.url)
                .unwrap_or_default();
            let names = self
                .evaluate_utility(
                    &PAGE_NAMES_SCRIPT.replace("MAX_NAMES", &MAX_PAGE_NAMES.to_string()),
                )
                .await
                .ok()
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default();
            let PageNames { classes, ids } = names;
            let Some(css) = adblock.cosmetic_css(&url, classes, ids).await else {
                return;
            };
            let Ok(css) = serde_json::to_string(&css) else {
                return;
            };

            let script = COSMETIC_SCRIPT.replace("CSS", &css);
            if let Err(error) = self.evaluate_utility(&script).await {
                debug!(%error, "cosmetic rules not applied");
            }
        });
    }
}

// Every Tab that follows the defaults gets them now; the others keep their own.
async fn apply_defaults(pages: Vec<Page>, options: &ContentOptions) -> Result<(), Error> {
    let mut first_error = None;
    for page in pages {
        if page.state.set_defaults(Some(options.clone()))
            && let Err(error) = page.apply_content().await
        {
            first_error.get_or_insert(error);
        }
    }

    first_error.map_or(Ok(()), Err)
}

impl Driver {
    pub async fn set_content_defaults(&self, options: ContentOptions) -> Result<(), Error> {
        let pages = match self {
            Driver::Chromium(chromium) | Driver::Live(Live { chromium, .. }) => {
                chromium.adopt_content_defaults(&options)
            }
            Driver::Camoufox(camoufox) => camoufox.adopt_content_defaults(&options),
        };

        if let Driver::Camoufox(camoufox) = self {
            camoufox.set_context_interception(None, &options).await?;
        }

        apply_defaults(pages, &options).await
    }
}
