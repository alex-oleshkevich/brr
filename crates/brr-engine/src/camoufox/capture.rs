use base64::Engine;
use brr_core::Error;
use brr_core::capture::ImageFormat;
use serde_json::{Value, json};

use super::PageSession;

impl PageSession {
    // `clip` is a document rectangle in CSS pixels; Juggler cannot capture without one and
    // rejects the `scale` member Chromium's clip carries. It renders at the page's device scale
    // factor, as Chromium does.
    pub(crate) async fn screenshot(
        &self,
        clip: Value,
        format: ImageFormat,
        quality: Option<u8>,
    ) -> Result<Vec<u8>, Error> {
        let mime_type = match format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Webp => "image/webp",
        };
        let mut params = json!({
            "mimeType": mime_type,
            "clip": {
                "x": clip["x"],
                "y": clip["y"],
                "width": clip["width"],
                "height": clip["height"],
            },
        });
        if format != ImageFormat::Png
            && let Some(quality) = quality
        {
            params["quality"] = json!(quality);
        }

        let mut result = self.call("Page.screenshot", params).await?;

        let Value::String(data) = result["data"].take() else {
            return Err(Error::Internal {
                message: "Page.screenshot returned no data".to_owned(),
            });
        };

        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| Error::Internal {
                message: format!("the browser sent invalid base64: {error}"),
            })
    }
}
