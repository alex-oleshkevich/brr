use brr_core::{Error, TargetId};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use brr_proto::CONNECTION_CLOSED;

use crate::{Juggler, SessionId};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateResult {
    pub navigation_id: Option<String>,
}

#[derive(Deserialize)]
struct Info {
    version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    target_id: TargetId,
}

fn parse<T: DeserializeOwned>(method: &str, result: Value) -> Result<T, Error> {
    serde_json::from_value(result).map_err(|error| Error::Internal {
        message: format!("{method}: unexpected response: {error}"),
    })
}

impl Juggler {
    // Must precede new_page: without attachToDefaultContext the default context's pages never attach.
    pub async fn enable(&self, user_prefs: &Map<String, Value>) -> Result<(), Error> {
        let prefs: Vec<Value> = user_prefs
            .iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect();
        let params = json!({ "attachToDefaultContext": true, "userPrefs": prefs });
        self.call(None, "Browser.enable", params).await?;

        Ok(())
    }

    pub async fn browser_version(&self) -> Result<String, Error> {
        let method = "Browser.getInfo";
        let result = self.call(None, method, json!({})).await?;

        Ok(parse::<Info>(method, result)?.version)
    }

    pub async fn new_page(&self, browser_context_id: Option<&str>) -> Result<TargetId, Error> {
        let method = "Browser.newPage";
        let params = match browser_context_id {
            Some(browser_context_id) => json!({ "browserContextId": browser_context_id }),
            None => json!({}),
        };
        let result = self.call(None, method, params).await?;

        Ok(parse::<Created>(method, result)?.target_id)
    }

    pub async fn close_page(&self, session_id: &SessionId) -> Result<(), Error> {
        self.call(
            Some(session_id),
            "Page.close",
            json!({ "runBeforeUnload": false }),
        )
        .await?;

        Ok(())
    }

    pub async fn navigate(
        &self,
        session_id: &SessionId,
        frame_id: &str,
        url: &str,
    ) -> Result<NavigateResult, Error> {
        let method = "Page.navigate";
        let params = json!({ "frameId": frame_id, "url": url });
        let result = self.call(Some(session_id), method, params).await?;

        parse(method, result)
    }

    // Juggler never answers Browser.close; it shuts the pipe down instead.
    pub async fn close_browser(&self) -> Result<(), Error> {
        match self.call(None, "Browser.close", json!({})).await {
            Err(Error::Internal { message }) if message == CONNECTION_CLOSED => Ok(()),
            result => result.map(|_| ()),
        }
    }
}
