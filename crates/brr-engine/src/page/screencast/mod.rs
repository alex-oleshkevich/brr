use std::sync::{Arc, PoisonError};

use base64::Engine;
use brr_core::{Error, Size};
use serde_json::Value;
use tokio::sync::watch;
use tracing::debug;

use super::{Page, PageState, Session};

#[cfg(test)]
mod tests;

const SCREENCAST_FIELD: &str = "screencast";
// A JPEG's dimensions sit in the first marker segments; 64 KiB covers any real header. A multiple
// of four, so the prefix decodes on its own.
const HEADER_BASE64_CHARS: usize = 64 * 1024 / 3 * 4 + 4;
const SOI: [u8; 2] = [0xFF, 0xD8];
const MARKER_PREFIX: u8 = 0xFF;
const SOF_FIRST: u8 = 0xC0;
const SOF_LAST: u8 = 0xCF;
// In the SOF range, but not frame headers.
const NOT_FRAMES: [u8; 3] = [0xC4, 0xC8, 0xCC];
// Markers that stand alone, without a length.
const STANDALONE: std::ops::RangeInclusive<u8> = 0xD0..=0xD9;
const TEM: u8 = 0x01;
const SOF_HEIGHT_AT: usize = 3;
const SOF_WIDTH_AT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameMetadata {
    pub device_width: f64,
    pub device_height: f64,
    pub page_scale_factor: f64,
    pub offset_top: f64,
}

// The image is the browser's base64 string, shared by every viewer and never decoded. An
// `Arc<String>` takes the received buffer over where an `Arc<str>` would copy it.
#[derive(Debug, Clone)]
pub struct ScreencastFrame {
    pub data: Arc<String>,
    pub width: u32,
    pub height: u32,
    pub metadata: FrameMetadata,
}

// The size from the first frame header (SOF0..SOF15 but DHT, JPG and DAC); None when the bytes
// end or break before one.
pub fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&SOI) {
        return None;
    }
    let mut at = SOI.len();

    loop {
        while *bytes.get(at)? == MARKER_PREFIX && *bytes.get(at + 1)? == MARKER_PREFIX {
            at += 1;
        }
        if *bytes.get(at)? != MARKER_PREFIX {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        at += 2;
        if STANDALONE.contains(&marker) || marker == TEM {
            continue;
        }

        let length = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        if (SOF_FIRST..=SOF_LAST).contains(&marker) && !NOT_FRAMES.contains(&marker) {
            let word = |offset: usize| -> Option<u32> {
                Some(u32::from(u16::from_be_bytes([
                    *bytes.get(at + offset)?,
                    *bytes.get(at + offset + 1)?,
                ])))
            };

            return Some((word(SOF_WIDTH_AT)?, word(SOF_HEIGHT_AT)?));
        }
        if length < 2 {
            return None;
        }
        at += length;
    }
}

// Frames go to one viewer stream at a time, and only the latest matters: a stream that falls
// behind skips to the newest frame instead of queueing the old ones.
pub(crate) type FrameSender = watch::Sender<Option<ScreencastFrame>>;

// What a running screencast hands out. Dropping it stops the browser's screencast, also when
// the call that started it was cancelled.
pub struct Screencast {
    frames: watch::Receiver<Option<ScreencastFrame>>,
    page: Page,
}

impl Screencast {
    // The newest frame not yet seen; None once the Tab is gone.
    pub async fn next(&mut self) -> Option<ScreencastFrame> {
        self.frames.changed().await.ok()?;

        self.frames.borrow_and_update().clone()
    }
}

impl Drop for Screencast {
    // The slot is freed here and the stop is sent after it, so a restart cannot be undone by
    // the stop of the stream before it: a start waits for the operation lock the stop holds.
    fn drop(&mut self) {
        let page = self.page.clone();
        page.state.release_screencast();
        let operation = page.state.screencast_operation.clone();
        let held = operation.clone().try_lock_owned().ok();

        tokio::spawn(async move {
            let _held = match held {
                Some(held) => held,
                None => operation.lock_owned().await,
            };
            let _ = match &page.session {
                Session::Chromium(session) => session.stop_screencast().await,
                Session::Camoufox(session) => session.stop_screencast().await,
            };
        });
    }
}

impl PageState {
    fn claim_screencast(&self) -> Result<watch::Receiver<Option<ScreencastFrame>>, Error> {
        let mut sink = self
            .screencast
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if sink.is_some() {
            return Err(Error::InvalidArgument {
                field: SCREENCAST_FIELD.to_owned(),
                message: "already running".to_owned(),
            });
        }
        let (frames, receiver) = watch::channel(None);
        *sink = Some(frames);

        Ok(receiver)
    }

    // Ends the stream: its receiver sees the sender go.
    pub(crate) fn release_screencast(&self) {
        self.screencast
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    // Replaces the newest frame; never waits. The browser is acked either way.
    pub(crate) fn screencast_frame(&self, data: String, metadata: FrameMetadata) {
        let sink = self
            .screencast
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(sink) = sink.as_ref().filter(|sink| sink.receiver_count() > 0) else {
            return;
        };

        let header = &data.as_bytes()[..data.len().min(HEADER_BASE64_CHARS)];
        let mut decoded = Vec::new();
        let size = base64::engine::general_purpose::STANDARD
            .decode_vec(header, &mut decoded)
            .ok()
            .and_then(|()| jpeg_size(&decoded));
        let Some((width, height)) = size else {
            debug!("screencast frame dropped: no readable JPEG header");
            return;
        };

        sink.send_replace(Some(ScreencastFrame {
            data: Arc::new(data),
            width,
            height,
            metadata,
        }));
    }
}

// The number at `key` in a screencast metadata object.
pub(crate) fn number(object: &Value, key: &str) -> f64 {
    object[key].as_f64().unwrap_or_default()
}

impl Page {
    pub async fn start_screencast(&self, max: Size, quality: u8) -> Result<Screencast, Error> {
        let _operation = self.state.screencast_operation.lock().await;
        let frames = self.state.claim_screencast()?;
        // Built before the browser is asked, so a failure or a cancelled call drops it and the
        // slot is freed and a stop sent.
        let screencast = Screencast {
            frames,
            page: self.clone(),
        };
        match &self.session {
            Session::Chromium(session) => session.start_screencast(&max, quality).await?,
            Session::Camoufox(session) => session.start_screencast(&max, quality).await?,
        }

        Ok(screencast)
    }
}
