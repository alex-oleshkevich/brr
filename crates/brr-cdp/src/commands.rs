use brr_core::{Error, TargetId};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{Cdp, SessionId};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateResult {
    pub frame_id: String,
    pub loader_id: Option<String>,
    pub error_text: Option<String>,
}

#[derive(Deserialize)]
struct Version {
    product: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    target_id: TargetId,
}

#[derive(Deserialize)]
struct Evaluated {
    result: EvaluatedValue,
}

#[derive(Deserialize)]
struct EvaluatedValue {
    value: String,
}

fn parse<T: DeserializeOwned>(method: &str, result: Value) -> Result<T, Error> {
    serde_json::from_value(result).map_err(|error| Error::Internal {
        message: format!("{method}: unexpected response: {error}"),
    })
}

impl Cdp {
    pub async fn browser_version(&self) -> Result<String, Error> {
        let method = "Browser.getVersion";
        let result = self.call(None, method, json!({})).await?;

        Ok(parse::<Version>(method, result)?.product)
    }

    pub async fn set_discover_targets(&self) -> Result<(), Error> {
        self.call(
            None,
            "Target.setDiscoverTargets",
            json!({ "discover": true }),
        )
        .await?;

        Ok(())
    }

    pub async fn set_auto_attach(&self, session_id: Option<&SessionId>) -> Result<(), Error> {
        let params = json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true });
        self.call(session_id, "Target.setAutoAttach", params)
            .await?;

        Ok(())
    }

    pub async fn run_if_waiting(&self, session_id: &SessionId) -> Result<(), Error> {
        self.call(
            Some(session_id),
            "Runtime.runIfWaitingForDebugger",
            json!({}),
        )
        .await?;

        Ok(())
    }

    pub async fn enable_page(&self, session_id: &SessionId) -> Result<(), Error> {
        self.call(Some(session_id), "Page.enable", json!({}))
            .await?;
        self.call(
            Some(session_id),
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        )
        .await?;

        Ok(())
    }

    pub async fn create_target(&self, url: &str, new_window: bool) -> Result<TargetId, Error> {
        let method = "Target.createTarget";
        let params = json!({ "url": url, "newWindow": new_window });
        let result = self.call(None, method, params).await?;

        Ok(parse::<Created>(method, result)?.target_id)
    }

    pub async fn close_target(&self, target_id: &TargetId) -> Result<(), Error> {
        self.call(None, "Target.closeTarget", json!({ "targetId": target_id }))
            .await?;

        Ok(())
    }

    pub async fn activate_target(&self, target_id: &TargetId) -> Result<(), Error> {
        self.call(
            None,
            "Target.activateTarget",
            json!({ "targetId": target_id }),
        )
        .await?;

        Ok(())
    }

    pub async fn navigate(
        &self,
        session_id: &SessionId,
        url: &str,
    ) -> Result<NavigateResult, Error> {
        let method = "Page.navigate";
        let result = self
            .call(Some(session_id), method, json!({ "url": url }))
            .await?;

        parse(method, result)
    }

    pub async fn ready_state(&self, session_id: &SessionId) -> Result<String, Error> {
        let method = "Runtime.evaluate";
        let params = json!({ "expression": "document.readyState", "returnByValue": true });
        let result = self.call(Some(session_id), method, params).await?;

        Ok(parse::<Evaluated>(method, result)?.result.value)
    }

    pub async fn close_browser(&self) -> Result<(), Error> {
        self.call(None, "Browser.close", json!({})).await?;

        Ok(())
    }
}
