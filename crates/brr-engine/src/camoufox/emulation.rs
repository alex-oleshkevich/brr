use std::collections::BTreeMap;

use brr_core::capture::ColorScheme;
use brr_core::{Error, Size};
use serde_json::{Map, Value, json};

use super::PageSession;

impl PageSession {
    pub(crate) async fn set_viewport(
        &self,
        size: &Size,
        scale: Option<f64>,
        mobile: bool,
    ) -> Result<(), Error> {
        let mut params = json!({
            "viewportSize": { "width": size.width, "height": size.height },
            "isMobile": mobile,
        });
        if let Some(scale) = scale {
            params["deviceScaleFactor"] = json!(scale);
        }
        self.call("Page.setViewportSize", params).await?;

        Ok(())
    }

    // Juggler resets what is left out: callers pass every setting the Tab should have.
    pub(crate) async fn set_media(
        &self,
        color_scheme: Option<ColorScheme>,
        reduced_motion: Option<bool>,
    ) -> Result<(), Error> {
        let mut params = Map::new();
        if let Some(scheme) = color_scheme {
            let scheme = match scheme {
                ColorScheme::Dark => "dark",
                ColorScheme::Light => "light",
            };
            params.insert("colorScheme".to_owned(), json!(scheme));
        }
        if let Some(reduce) = reduced_motion {
            let motion = if reduce { "reduce" } else { "no-preference" };
            params.insert("reducedMotion".to_owned(), json!(motion));
        }
        self.call("Page.setEmulatedMedia", Value::Object(params))
            .await?;

        Ok(())
    }

    pub(crate) async fn set_extra_headers(
        &self,
        headers: &BTreeMap<String, String>,
    ) -> Result<(), Error> {
        let headers: Vec<Value> = headers
            .iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect();
        self.call("Network.setExtraHTTPHeaders", json!({ "headers": headers }))
            .await?;

        Ok(())
    }
}
