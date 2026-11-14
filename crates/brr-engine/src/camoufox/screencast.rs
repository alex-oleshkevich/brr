use std::sync::Arc;

use brr_core::{Error, Size};
use serde_json::{Value, json};
use tracing::debug;

use super::PageSession;
use crate::page::{FrameMetadata, PageState};

const DEFAULT_PAGE_SCALE: f64 = 1.0;

impl PageSession {
    pub(crate) async fn start_screencast(&self, max: &Size, quality: u8) -> Result<(), Error> {
        self.call(
            "Page.startScreencast",
            json!({ "width": max.width, "height": max.height, "quality": quality }),
        )
        .await?;

        Ok(())
    }

    pub(crate) async fn stop_screencast(&self) -> Result<(), Error> {
        self.call("Page.stopScreencast", json!({})).await?;

        Ok(())
    }
}

// Juggler allows one frame in flight, so the ack goes out first and never waits on the consumer.
// The ack's screencast id is optional and omitted: one screencast runs per Tab.
pub(super) fn frame(session: PageSession, state: Arc<PageState>, mut params: Value) {
    tokio::spawn({
        let session = session.clone();

        async move {
            if let Err(error) = session.call("Page.screencastFrameAck", json!({})).await {
                debug!(%error, "screencast frame not acked");
            }
        }
    });

    let Value::String(data) = params["data"].take() else {
        return;
    };
    state.screencast_frame(
        data,
        FrameMetadata {
            device_width: params["deviceWidth"].as_f64().unwrap_or_default(),
            device_height: params["deviceHeight"].as_f64().unwrap_or_default(),
            page_scale_factor: DEFAULT_PAGE_SCALE,
            offset_top: 0.0,
        },
    );
}
