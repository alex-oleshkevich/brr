use std::time::Duration;

use base64::Engine;
use brr_cdp::{Cdp, SessionId};
use brr_core::capture::{Area, ImageFormat, PdfOptions};
use brr_core::{Capability, Error};
use serde_json::{Value, json};

use super::emulation::pdf_params;
use super::{DEFAULT_EVALUATE_TIMEOUT, Element, Page, Session, Target};
use crate::camoufox;

pub const MAX_CAPTURE_BYTES: usize = 256 * 1024 * 1024;

const MHTML_TIMEOUT: Duration = Duration::from_secs(15);
const MHTML_FIELD: &str = "format";
const MHTML_TOO_SLOW: &str = "the snapshot did not arrive in time; the page may be larger than 256 MiB, use pdf or a screenshot instead";
const PDF_TIMEOUT: Duration = Duration::from_secs(300);
const PDF_CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_CAPTURE_SIDE: f64 = 16_384.0;
const BYTES_PER_PIXEL: f64 = 4.0;
const BASE64_EXPANSION: f64 = 4.0 / 3.0;
const DEVICE_PIXEL_RATIO: &str = "window.devicePixelRatio";
const AREA_FIELD: &str = "area";
const PAGES_FIELD: &str = "pages";
const PRINT_FAILED: &str = "Printing failed";
const TRANSPARENT_FIELD: &str = "transparent";
// Juggler needs a clip, in page coordinates, for every capture.
const VIEWPORT_RECT: &str = "({ x: scrollX, y: scrollY, width: innerWidth, height: innerHeight })";
const DOCUMENT_RECT: &str = "({ x: 0, y: 0, width: Math.max(document.documentElement.scrollWidth, innerWidth), height: Math.max(document.documentElement.scrollHeight, innerHeight) })";
const SCROLL_OFFSET: &str = "({ x: scrollX, y: scrollY })";
const TOO_LARGE: &str = "capture larger than 256 MiB; capture a smaller area";

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq)]
pub enum Clip {
    Viewport,
    FullPage,
    Element(Target),
    Area(Area),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotOptions {
    pub clip: Clip,
    pub format: ImageFormat,
    pub quality: Option<u8>,
    pub transparent: bool,
}

fn too_large() -> Error {
    Error::InvalidArgument {
        field: AREA_FIELD.to_owned(),
        message: TOO_LARGE.to_owned(),
    }
}

// A message over the transport cap is dropped and its call only times out, and Chromium cannot
// render past its texture limit: refuse a clip that big before capturing it. The image is the
// clip times the device scale factor, base64 encoded on the wire.
fn clip_json(x: f64, y: f64, width: f64, height: f64, scale: f64) -> Result<Value, Error> {
    let (pixels_wide, pixels_high) = (width * scale, height * scale);
    if pixels_wide > MAX_CAPTURE_SIDE
        || pixels_high > MAX_CAPTURE_SIDE
        || pixels_wide * pixels_high * BYTES_PER_PIXEL * BASE64_EXPANSION > MAX_CAPTURE_BYTES as f64
    {
        return Err(too_large());
    }

    Ok(json!({ "x": x, "y": y, "width": width, "height": height, "scale": 1 }))
}

// Chrome reports a document it cannot lay out as pages ("Printing failed") without naming a
// limit, and a very long one is the usual cause.
fn print_error(error: Error) -> Error {
    match error {
        Error::Internal { message } if message.contains(PRINT_FAILED) => Error::InvalidArgument {
            field: PAGES_FIELD.to_owned(),
            message: "the browser could not print this page; print fewer pages with page_ranges"
                .to_owned(),
        },
        other => other,
    }
}

// The stream's chunks, refused before one is decoded or appended past `limit`: growing past it
// would first reallocate the whole document.
async fn read_stream(
    cdp: &Cdp,
    session_id: &SessionId,
    handle: &str,
    limit: usize,
) -> Result<Vec<u8>, Error> {
    let mut pdf = Vec::new();

    loop {
        let mut chunk = cdp
            .call(
                Some(session_id),
                "IO.read",
                json!({ "handle": handle, "size": PDF_CHUNK_BYTES }),
            )
            .await?;
        let data = chunk["data"].take();
        let data = data.as_str().unwrap_or_default();
        let encoded = chunk["base64Encoded"].as_bool().unwrap_or(false);
        let incoming = if encoded {
            data.len() / 4 * 3
        } else {
            data.len()
        };
        if pdf.len() + incoming > limit {
            return Err(too_large());
        }

        if encoded {
            base64::engine::general_purpose::STANDARD
                .decode_vec(data, &mut pdf)
                .map_err(|error| Error::Internal {
                    message: format!("the browser sent invalid base64: {error}"),
                })?;
        } else {
            pdf.extend_from_slice(data.as_bytes());
        }
        if chunk["eof"].as_bool().unwrap_or(false) {
            return Ok(pdf);
        }
    }
}

fn rect(value: Value, scale: f64) -> Result<Value, Error> {
    let number = |name: &str| value[name].as_f64().unwrap_or_default();

    clip_json(
        number("x"),
        number("y"),
        number("width"),
        number("height"),
        scale,
    )
}

fn decode_image(data: &Value) -> Result<Vec<u8>, Error> {
    base64::engine::general_purpose::STANDARD
        .decode(data.as_str().unwrap_or_default())
        .map_err(|error| Error::Internal {
            message: format!("the browser sent invalid base64: {error}"),
        })
}

impl Page {
    // Set, capture and reset run in their own task: a stuck transparent background would taint
    // every later screenshot of this Tab, and a cancelled call must not leave it.
    pub async fn screenshot(&self, options: &ScreenshotOptions) -> Result<Vec<u8>, Error> {
        if let Session::Camoufox(session) = &self.session {
            // Juggler has no background override.
            if options.transparent {
                return Err(Error::InvalidArgument {
                    field: TRANSPARENT_FIELD.to_owned(),
                    message: "not available on Camoufox".to_owned(),
                });
            }

            return self.capture_firefox(session, options).await;
        }
        if !options.transparent {
            return self.capture(options).await;
        }

        let page = self.clone();
        let options = options.clone();
        let task = tokio::spawn(async move {
            page.page_call(
                "Emulation.setDefaultBackgroundColorOverride",
                json!({ "color": { "r": 0, "g": 0, "b": 0, "a": 0 } }),
            )
            .await?;
            let result = page.capture(&options).await;
            let reset = page
                .page_call("Emulation.setDefaultBackgroundColorOverride", json!({}))
                .await;

            let image = result?;
            reset?;

            Ok(image)
        });

        task.await.map_err(|error| Error::Internal {
            message: format!("screenshot task failed: {error}"),
        })?
    }

    // What the page renders at: a device descriptor or `scale` emulation changes it.
    async fn device_scale(&self) -> Result<f64, Error> {
        let ratio = self.evaluate_utility(DEVICE_PIXEL_RATIO).await?;

        Ok(ratio.as_f64().filter(|ratio| *ratio > 0.0).unwrap_or(1.0))
    }

    async fn capture(&self, options: &ScreenshotOptions) -> Result<Vec<u8>, Error> {
        let scale = self.device_scale().await?;
        let (clip, beyond_viewport) = match &options.clip {
            Clip::Viewport => (None, false),
            Clip::FullPage => {
                let metrics = self.page_call("Page.getLayoutMetrics", json!({})).await?;
                let size = &metrics["cssContentSize"];
                // The content width excludes a vertical scrollbar; the capture is as wide as
                // the viewport.
                let viewport = self.evaluate_utility("innerWidth").await?;
                let width = size["width"]
                    .as_f64()
                    .unwrap_or_default()
                    .max(viewport.as_f64().unwrap_or_default());

                (
                    Some(clip_json(
                        0.0,
                        0.0,
                        width,
                        size["height"].as_f64().unwrap_or_default(),
                        scale,
                    )?),
                    true,
                )
            }
            Clip::Element(target) => (Some(self.element_clip(target, scale).await?), false),
            Clip::Area(area) => (
                Some(clip_json(area.x, area.y, area.width, area.height, scale)?),
                true,
            ),
        };

        let format = match options.format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Webp => "webp",
        };
        // Always explicit: the flag can leave the layout changed until the next capture.
        let mut params = json!({
            "format": format,
            "captureBeyondViewport": beyond_viewport,
            "fromSurface": true,
        });
        if options.format != ImageFormat::Png
            && let Some(quality) = options.quality
        {
            params["quality"] = json!(quality);
        }
        if let Some(clip) = clip {
            params["clip"] = clip;
        }

        let mut result = self.page_call("Page.captureScreenshot", params).await?;

        decode_image(&result["data"].take())
    }

    async fn capture_firefox(
        &self,
        session: &camoufox::PageSession,
        options: &ScreenshotOptions,
    ) -> Result<Vec<u8>, Error> {
        let scale = self.device_scale().await?;
        let clip = match &options.clip {
            Clip::Viewport => rect(self.evaluate_utility(VIEWPORT_RECT).await?, scale)?,
            Clip::FullPage => rect(self.evaluate_utility(DOCUMENT_RECT).await?, scale)?,
            Clip::Element(target) => self.element_clip(target, scale).await?,
            Clip::Area(area) => clip_json(area.x, area.y, area.width, area.height, scale)?,
        };
        session
            .screenshot(clip, options.format, options.quality)
            .await
    }

    // The element's border quad in its own frame's viewport, and where that viewport sits on
    // the page.
    async fn element_quad(&self, element: &Element) -> Result<(Vec<f64>, f64, f64), Error> {
        if let Session::Camoufox(_) = &self.session {
            let quads = self.session.content_quads(element).await;
            self.release(element);
            let scroll = self.evaluate_utility(SCROLL_OFFSET).await?;
            let quad = quads?.into_iter().next().unwrap_or_default();

            return Ok((
                quad.to_vec(),
                scroll["x"].as_f64().unwrap_or_default(),
                scroll["y"].as_f64().unwrap_or_default(),
            ));
        }

        let model = self
            .session
            .cdp()?
            .call(
                Some(&element.session_id),
                "DOM.getBoxModel",
                json!({ "objectId": element.object_id }),
            )
            .await;
        self.release(element);
        let model = model?;
        let metrics = self.page_call("Page.getLayoutMetrics", json!({})).await?;
        let viewport = &metrics["cssVisualViewport"];
        let quad = model["model"]["border"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .collect();

        Ok((
            quad,
            viewport["pageX"].as_f64().unwrap_or_default(),
            viewport["pageY"].as_f64().unwrap_or_default(),
        ))
    }

    async fn element_clip(&self, target: &Target, scale: f64) -> Result<Value, Error> {
        self.scroll_into_view(target, DEFAULT_EVALUATE_TIMEOUT)
            .await?;
        let element = self.resolve(target, DEFAULT_EVALUATE_TIMEOUT).await?;
        let (quad, page_x, page_y) = self.element_quad(&element).await?;
        let xs = quad.iter().step_by(2).copied();
        let ys = quad.iter().skip(1).step_by(2).copied();
        let (min_x, max_x) = xs.fold((f64::MAX, f64::MIN), |(min, max), x| {
            (min.min(x), max.max(x))
        });
        let (min_y, max_y) = ys.fold((f64::MAX, f64::MIN), |(min, max), y| {
            (min.min(y), max.max(y))
        });
        let (width, height) = (max_x - min_x, max_y - min_y);
        if quad.len() < 8 || width <= 0.0 || height <= 0.0 {
            return Err(Error::ElementNotFound);
        }

        // The quad is relative to the viewport of the element's own process: add the iframes
        // above it, then the scroll position, since the clip is in page coordinates.
        let root = self.state.local_root(&element.frame_id);
        let offsets = self.frame_offsets(&root).await?;
        let shift_x: f64 = offsets.iter().map(|offset| offset.x).sum();
        let shift_y: f64 = offsets.iter().map(|offset| offset.y).sum();

        clip_json(
            min_x + shift_x + page_x,
            min_y + shift_y + page_y,
            width,
            height,
            scale,
        )
    }

    pub async fn pdf(&self, options: &PdfOptions) -> Result<Vec<u8>, Error> {
        if let Session::Camoufox(_) = &self.session {
            return Err(Error::EngineUnsupported {
                capability: Capability::Pdf,
            });
        }
        // Streamed: an inline result of a large document hits the message size cap.
        let printed = self
            .session
            .cdp()?
            .call_within(
                Some(self.session.main_session()),
                "Page.printToPDF",
                pdf_params(options)?,
                PDF_TIMEOUT,
            )
            .await
            .map_err(print_error)?;
        let handle = printed["stream"].as_str().ok_or_else(|| Error::Internal {
            message: "Page.printToPDF returned no stream".to_owned(),
        })?;

        // Its own task: a cancelled caller must not leave the stream open in the browser.
        let cdp = self.session.cdp()?.clone();
        let session_id = self.session.main_session().clone();
        let handle = handle.to_owned();
        tokio::spawn(async move {
            let read = read_stream(&cdp, &session_id, &handle, MAX_CAPTURE_BYTES).await;
            let _ = cdp
                .call(Some(&session_id), "IO.close", json!({ "handle": handle }))
                .await;

            read
        })
        .await
        .map_err(|error| Error::Internal {
            message: format!("pdf task failed: {error}"),
        })?
    }

    pub async fn mhtml(&self) -> Result<Vec<u8>, Error> {
        if let Session::Camoufox(_) = &self.session {
            return Err(Error::EngineUnsupported {
                capability: Capability::Mhtml,
            });
        }
        // A snapshot over the transport cap is dropped by the reader and never answered.
        let mut snapshot = self
            .session
            .cdp()?
            .call_within(
                Some(self.session.main_session()),
                "Page.captureSnapshot",
                json!({ "format": "mhtml" }),
                MHTML_TIMEOUT,
            )
            .await
            .map_err(|error| match error {
                Error::Timeout { .. } => Error::InvalidArgument {
                    field: MHTML_FIELD.to_owned(),
                    message: MHTML_TOO_SLOW.to_owned(),
                },
                other => other,
            })?;

        match snapshot["data"].take() {
            Value::String(data) => Ok(data.into_bytes()),
            _ => Ok(Vec::new()),
        }
    }
}
