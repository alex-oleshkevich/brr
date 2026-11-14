use brr_core::{Error, TargetId};
use serde_json::json;

use super::Chromium;
use crate::ContextId;
use crate::page::PageSession;

const GEOLOCATION: &str = "geolocation";

impl Chromium {
    pub(crate) async fn create_context(&self) -> Result<ContextId, Error> {
        let created = self
            .inner
            .cdp
            .call(
                None,
                "Target.createBrowserContext",
                json!({ "disposeOnDetach": false }),
            )
            .await?;
        let context_id = created["browserContextId"]
            .as_str()
            .ok_or_else(|| super::internal("Target.createBrowserContext: no browserContextId"))?
            .to_owned();

        let behavior = self
            .inner
            .cdp
            .call(
                None,
                "Browser.setDownloadBehavior",
                json!({
                    "behavior": "allowAndName",
                    "browserContextId": context_id,
                    "downloadPath": self.inner.download_dir,
                    "eventsEnabled": true,
                }),
            )
            .await;
        // Nobody holds the id yet: a context left behind could only be disposed with the browser.
        if let Err(error) = behavior {
            let _ = self.dispose_context(&ContextId(context_id)).await;

            return Err(error);
        }

        Ok(ContextId(context_id))
    }

    pub(crate) async fn open_tab_in(
        &self,
        context_id: &ContextId,
        url: &str,
    ) -> Result<TargetId, Error> {
        let created = self
            .inner
            .cdp
            .call(
                None,
                "Target.createTarget",
                json!({ "url": url, "browserContextId": context_id.0, "newWindow": true }),
            )
            .await?;

        created["targetId"]
            .as_str()
            .map(|id| TargetId(id.to_owned()))
            .ok_or_else(|| super::internal("Target.createTarget: no targetId"))
    }

    // Its Tabs report TabDestroyed through the normal event path.
    pub(crate) async fn dispose_context(&self, context_id: &ContextId) -> Result<(), Error> {
        self.inner
            .cdp
            .call(
                None,
                "Target.disposeBrowserContext",
                json!({ "browserContextId": context_id.0 }),
            )
            .await?;

        Ok(())
    }
}

impl PageSession {
    // Permissions belong to a browser context, so the Tab's own is looked up.
    pub(crate) async fn grant_geolocation(&self) -> Result<(), Error> {
        let info = self
            .cdp
            .call(
                None,
                "Target.getTargetInfo",
                json!({ "targetId": self.target_id }),
            )
            .await?;

        self.cdp
            .call(
                None,
                "Browser.grantPermissions",
                json!({
                    "permissions": [GEOLOCATION],
                    "browserContextId": info["targetInfo"]["browserContextId"],
                }),
            )
            .await?;

        Ok(())
    }
}
