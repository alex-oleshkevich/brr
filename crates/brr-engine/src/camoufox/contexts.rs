use brr_core::{Error, TargetId};
use serde_json::json;

use super::{Camoufox, downloads, internal};
use crate::ContextId;

impl Camoufox {
    pub(crate) async fn create_context(&self) -> Result<ContextId, Error> {
        let created = self
            .inner
            .juggler
            .call(
                None,
                "Browser.createBrowserContext",
                json!({ "removeOnDetach": false }),
            )
            .await?;
        let context_id = ContextId(
            created["browserContextId"]
                .as_str()
                .ok_or_else(|| internal("Browser.createBrowserContext: no browserContextId"))?
                .to_owned(),
        );

        let configured = downloads::configure(
            &self.inner.juggler,
            Some(&context_id.0),
            &self.inner.download_dir,
        )
        .await;
        // Nobody holds the id yet: a context left behind could only be disposed with the browser.
        if let Err(error) = configured {
            let _ = self.dispose_context(&context_id).await;

            return Err(error);
        }

        let defaults = self
            .inner
            .content_defaults
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(defaults) = defaults
            && let Err(error) = self
                .set_context_interception(Some(&context_id.0), &defaults)
                .await
        {
            let _ = self.dispose_context(&context_id).await;

            return Err(error);
        }

        Ok(context_id)
    }

    pub(crate) async fn open_tab_in(
        &self,
        context_id: &ContextId,
        url: &str,
    ) -> Result<TargetId, Error> {
        self.open_tab_with(Some(&context_id.0), url).await
    }

    // Its Tabs report TabDestroyed through the normal event path.
    pub(crate) async fn dispose_context(&self, context_id: &ContextId) -> Result<(), Error> {
        self.inner
            .juggler
            .call(
                None,
                "Browser.removeBrowserContext",
                json!({ "browserContextId": context_id.0 }),
            )
            .await?;

        Ok(())
    }
}
