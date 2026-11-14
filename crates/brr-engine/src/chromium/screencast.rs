use std::sync::Arc;

use brr_cdp::{Cdp, SessionId};
use brr_core::{Error, Size};
use serde_json::{Value, json};
use tracing::debug;

use crate::page::{FrameMetadata, PageSession, PageState, number};

impl PageSession {
    pub(crate) async fn start_screencast(&self, max: &Size, quality: u8) -> Result<(), Error> {
        self.cdp
            .call(
                Some(&self.session_id),
                "Page.startScreencast",
                json!({
                    "format": "jpeg",
                    "quality": quality,
                    "maxWidth": max.width,
                    "maxHeight": max.height,
                    "everyNthFrame": 1,
                }),
            )
            .await?;

        Ok(())
    }

    pub(crate) async fn stop_screencast(&self) -> Result<(), Error> {
        self.cdp
            .call(Some(&self.session_id), "Page.stopScreencast", json!({}))
            .await?;

        Ok(())
    }
}

// Chromium sends no further frame until the last is acked, so the ack goes out first and never
// waits on the consumer: a slow viewer must not stop the stream for the others.
pub(super) fn frame(cdp: Cdp, session_id: SessionId, state: Arc<PageState>, mut params: Value) {
    let ack = params["sessionId"].clone();
    tokio::spawn({
        let session_id = session_id.clone();

        async move {
            if let Err(error) = cdp
                .call(
                    Some(&session_id),
                    "Page.screencastFrameAck",
                    json!({ "sessionId": ack }),
                )
                .await
            {
                debug!(%error, "screencast frame not acked");
            }
        }
    });

    let Value::String(data) = params["data"].take() else {
        return;
    };
    let metadata = &params["metadata"];
    state.screencast_frame(
        data,
        FrameMetadata {
            device_width: number(metadata, "deviceWidth"),
            device_height: number(metadata, "deviceHeight"),
            page_scale_factor: number(metadata, "pageScaleFactor"),
            offset_top: number(metadata, "offsetTop"),
        },
    );
}
