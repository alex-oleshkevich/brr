use std::collections::BTreeMap;

use brr_core::capture::{ColorScheme, PAPER_SIZES, PdfOptions, margins_in};
use brr_core::{Error, Geolocation, Size};
use serde_json::{Value, json};

use super::devices::{Device, device};
use super::{DEFAULT_EVALUATE_TIMEOUT, Page, Session};
use crate::camoufox;

#[cfg(test)]
mod tests;

const DEFAULT_PAPER: &str = "A4";
pub(super) const DEFAULT_MARGIN_IN: f64 = 10.0 / 25.4;
const DEFAULT_ACCURACY_M: f64 = 10.0;
const DEVICE_FIELD: &str = "device";
const SCALE_FIELD: &str = "scale";
const DEFAULT_SCALE: f64 = 1.0;
const TIMEZONE_FIELD: &str = "timezone";
const GEOLOCATION_FIELD: &str = "geolocation";
const VIEWPORT_SIZE: &str = "({ width: innerWidth, height: innerHeight })";
const OPAQUE_ORIGIN: &str = "null";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmulateOptions {
    pub viewport: Option<Size>,
    pub scale: Option<f64>,
    pub device: Option<String>,
    pub geolocation: Option<Geolocation>,
    pub timezone: Option<String>,
    pub locale: Option<String>,
    pub user_agent: Option<String>,
    pub offline: Option<bool>,
    pub color_scheme: Option<ColorScheme>,
    pub reduced_motion: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cleanup<'a> {
    pub hide: Option<&'a str>,
    pub remove: Option<&'a str>,
    pub css: Option<&'a str>,
    pub js: Option<&'a str>,
}

// What the Tab is emulating where one CDP call carries several fields: later calls change only
// what they name, so the rest is sent again from here.
#[derive(Debug, Default)]
pub(crate) struct Media {
    color_scheme: Option<ColorScheme>,
    reduced_motion: Option<bool>,
    user_agent: Option<String>,
    locale: Option<String>,
}

// printToPDF parameters; paper and margins are inches.
pub(crate) fn pdf_params(options: &PdfOptions) -> Result<Value, Error> {
    let paper = options.paper.as_deref().unwrap_or(DEFAULT_PAPER);
    let (_, width, height) = PAPER_SIZES
        .iter()
        .find(|(name, ..)| name.eq_ignore_ascii_case(paper))
        .ok_or_else(|| Error::InvalidArgument {
            field: "paper".to_owned(),
            message: format!("`{paper}` is not a known paper size"),
        })?;
    // Chromium rotates a page itself when `landscape` is set, on top of any swap done here, so
    // the sizes are swapped here and the flag is not sent.
    let (width, height) = if options.landscape.unwrap_or(false) {
        (*height, *width)
    } else {
        (*width, *height)
    };
    let [top, right, bottom, left] = match &options.margin {
        Some(margin) => margins_in(margin)?,
        None => [DEFAULT_MARGIN_IN; 4],
    };

    Ok(json!({
        "paperWidth": width,
        "paperHeight": height,
        "landscape": false,
        "marginTop": top,
        "marginRight": right,
        "marginBottom": bottom,
        "marginLeft": left,
        "scale": options.pdf_scale.unwrap_or(1.0),
        "pageRanges": options.pages.as_deref().unwrap_or(""),
        "printBackground": options.print_background.unwrap_or(false),
        "transferMode": "ReturnAsStream",
    }))
}

impl Page {
    pub async fn emulate(&self, options: &EmulateOptions) -> Result<(), Error> {
        let device = match &options.device {
            Some(name) => Some(device(name).ok_or_else(|| Error::InvalidArgument {
                field: DEVICE_FIELD.to_owned(),
                message: format!("`{name}` is not a known device name"),
            })?),
            None => None,
        };

        if let Session::Camoufox(session) = &self.session {
            return self.emulate_firefox(session, options, device).await;
        }

        let viewport = options
            .viewport
            .clone()
            .or_else(|| device.map(|device| device.viewport.clone()));
        let scale = options.scale.or_else(|| device.map(|device| device.scale));
        if viewport.is_some() || scale.is_some() {
            let size = viewport.unwrap_or(Size {
                width: 0,
                height: 0,
            });
            self.page_call(
                "Emulation.setDeviceMetricsOverride",
                json!({
                    "width": size.width,
                    "height": size.height,
                    "deviceScaleFactor": scale.unwrap_or(0.0),
                    "mobile": device.is_some_and(|device| device.mobile),
                }),
            )
            .await?;
        }
        if let Some(device) = device {
            self.page_call(
                "Emulation.setTouchEmulationEnabled",
                json!({ "enabled": device.touch }),
            )
            .await?;
        }

        // navigator.language comes from the accept-language of the UA override, which replaces
        // user agent and language together; an empty user agent keeps the browser's own.
        let ua_call = {
            let mut media = self
                .state
                .media
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let user_agent = options
                .user_agent
                .clone()
                .or_else(|| device.map(|device| device.user_agent.clone()));
            let changed = user_agent.is_some() || options.locale.is_some();
            if user_agent.is_some() {
                media.user_agent = user_agent;
            }
            if options.locale.is_some() {
                media.locale.clone_from(&options.locale);
            }

            changed.then(|| {
                json!({
                    "userAgent": media.user_agent.as_deref().unwrap_or(""),
                    "acceptLanguage": media.locale.as_deref().unwrap_or(""),
                })
            })
        };
        if let Some(params) = ua_call {
            self.page_call("Emulation.setUserAgentOverride", params)
                .await?;
        }
        if let Some(locale) = &options.locale {
            self.page_call("Emulation.setLocaleOverride", json!({ "locale": locale }))
                .await?;
        }
        if let Some(timezone) = &options.timezone {
            self.check_timezone(timezone).await?;
            self.page_call(
                "Emulation.setTimezoneOverride",
                json!({ "timezoneId": timezone }),
            )
            .await?;
        }

        if let Some(geolocation) = &options.geolocation {
            self.session.grant_geolocation().await?;
            self.page_call(
                "Emulation.setGeolocationOverride",
                json!({
                    "latitude": geolocation.latitude,
                    "longitude": geolocation.longitude,
                    "accuracy": geolocation.accuracy.unwrap_or(DEFAULT_ACCURACY_M),
                }),
            )
            .await?;
        }
        if let Some(offline) = options.offline {
            self.page_call(
                "Network.emulateNetworkConditions",
                json!({
                    "offline": offline,
                    "latency": 0,
                    "downloadThroughput": -1,
                    "uploadThroughput": -1,
                }),
            )
            .await?;
        }

        if options.color_scheme.is_some() || options.reduced_motion.is_some() {
            let features = {
                let mut media = self
                    .state
                    .media
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if options.color_scheme.is_some() {
                    media.color_scheme = options.color_scheme;
                }
                if options.reduced_motion.is_some() {
                    media.reduced_motion = options.reduced_motion;
                }
                let scheme = match media.color_scheme {
                    Some(ColorScheme::Dark) => "dark",
                    Some(ColorScheme::Light) => "light",
                    None => "",
                };
                let motion = if media.reduced_motion == Some(true) {
                    "reduce"
                } else {
                    "no-preference"
                };

                json!([
                    { "name": "prefers-color-scheme", "value": scheme },
                    { "name": "prefers-reduced-motion", "value": motion },
                ])
            };
            self.page_call(
                "Emulation.setEmulatedMedia",
                json!({ "features": features }),
            )
            .await?;
        }

        Ok(())
    }

    // Chromium accepts an unknown id silently while another override is active, so the id is
    // checked where Intl can reject it.
    async fn check_timezone(&self, timezone: &str) -> Result<(), Error> {
        let check = format!(
            "(() => {{ try {{ new Intl.DateTimeFormat(undefined, {{ timeZone: {} }}); return true; }} catch (error) {{ return false; }} }})()",
            json!(timezone)
        );
        if self.evaluate_utility(&check).await? == json!(true) {
            return Ok(());
        }

        Err(Error::InvalidArgument {
            field: TIMEZONE_FIELD.to_owned(),
            message: format!("`{timezone}` is not a known IANA time zone"),
        })
    }

    // Juggler overrides belong to the Tab's browser context, not to the Tab.
    async fn emulate_firefox(
        &self,
        session: &camoufox::PageSession,
        options: &EmulateOptions,
        device: Option<&Device>,
    ) -> Result<(), Error> {
        let viewport = options
            .viewport
            .clone()
            .or_else(|| device.map(|device| device.viewport.clone()));
        // Juggler's Page.setViewportSize ignores deviceScaleFactor, and Browser.setDefaultViewport
        // applies it to every Tab of the context: a scale is refused rather than dropped.
        if options.scale.is_some_and(|scale| scale != DEFAULT_SCALE) {
            return Err(Error::InvalidArgument {
                field: SCALE_FIELD.to_owned(),
                message: "not available on Camoufox".to_owned(),
            });
        }
        let scale = options.scale.or_else(|| device.map(|device| device.scale));
        if viewport.is_some() || scale.is_some() {
            // The size is required, so a scale alone keeps the current one.
            let size = match viewport {
                Some(size) => size,
                None => serde_json::from_value(self.evaluate_utility(VIEWPORT_SIZE).await?)
                    .map_err(|error| Error::Internal {
                        message: error.to_string(),
                    })?,
            };
            session
                .set_viewport(&size, scale, device.is_some_and(|device| device.mobile))
                .await?;
        }
        if let Some(device) = device {
            session
                .browser_call(
                    "Browser.setTouchOverride",
                    json!({ "hasTouch": device.touch }),
                )
                .await?;
        }

        let user_agent = options
            .user_agent
            .as_deref()
            .or_else(|| device.map(|device| device.user_agent.as_str()));
        if let Some(user_agent) = user_agent {
            session
                .browser_call(
                    "Browser.setUserAgentOverride",
                    json!({ "userAgent": user_agent }),
                )
                .await?;
        }
        if let Some(locale) = &options.locale {
            session
                .browser_call("Browser.setLocaleOverride", json!({ "locale": locale }))
                .await?;
        }
        if let Some(timezone) = &options.timezone {
            self.check_timezone(timezone).await?;
            session
                .browser_call(
                    "Browser.setTimezoneOverride",
                    json!({ "timezoneId": timezone }),
                )
                .await?;
        }

        if let Some(geolocation) = &options.geolocation {
            // Juggler grants a permission per origin, so only the page's own origin is covered.
            let origin = self
                .evaluate_internal("location.origin", DEFAULT_EVALUATE_TIMEOUT)
                .await?;
            let origin = origin.as_str().unwrap_or(OPAQUE_ORIGIN);
            if origin == OPAQUE_ORIGIN {
                return Err(Error::InvalidArgument {
                    field: GEOLOCATION_FIELD.to_owned(),
                    message: "open a page with a web origin first; permissions belong to origins"
                        .to_owned(),
                });
            }
            session
                .browser_call(
                    "Browser.grantPermissions",
                    json!({ "origin": origin, "permissions": ["geolocation"] }),
                )
                .await?;
            session
                .browser_call(
                    "Browser.setGeolocationOverride",
                    json!({ "geolocation": {
                        "latitude": geolocation.latitude,
                        "longitude": geolocation.longitude,
                        "accuracy": geolocation.accuracy.unwrap_or(DEFAULT_ACCURACY_M),
                    } }),
                )
                .await?;
        }
        if let Some(offline) = options.offline {
            let state = if offline {
                json!("offline")
            } else {
                Value::Null
            };
            session
                .browser_call("Browser.setOnlineOverride", json!({ "override": state }))
                .await?;
        }

        if options.color_scheme.is_some() || options.reduced_motion.is_some() {
            // Juggler resets what a call leaves out, so the Tab's earlier settings are sent again.
            let (color_scheme, reduced_motion) = {
                let mut media = self
                    .state
                    .media
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if options.color_scheme.is_some() {
                    media.color_scheme = options.color_scheme;
                }
                if options.reduced_motion.is_some() {
                    media.reduced_motion = options.reduced_motion;
                }

                (media.color_scheme, media.reduced_motion)
            };
            session.set_media(color_scheme, reduced_motion).await?;
        }

        Ok(())
    }

    // Hide and remove take selectors, so they are checked first and name their field.
    pub async fn cleanup(&self, cleanup: &Cleanup<'_>) -> Result<(), Error> {
        if let Some(selector) = cleanup.hide {
            let style = format!("{selector}{{visibility:hidden !important}}");
            self.checked_selector("hide", selector, &add_style(&style))
                .await?;
        }
        if let Some(selector) = cleanup.remove {
            let remove = format!(
                "document.querySelectorAll({}).forEach((element) => element.remove())",
                json!(selector)
            );
            self.checked_selector("remove", selector, &remove).await?;
        }
        if let Some(css) = cleanup.css {
            self.evaluate_utility(&add_style(css)).await?;
        }
        if let Some(js) = cleanup.js {
            self.evaluate(js, DEFAULT_EVALUATE_TIMEOUT).await?;
        }

        Ok(())
    }

    // Runs `action` in the utility world once the selector parses.
    async fn checked_selector(
        &self,
        field: &str,
        selector: &str,
        action: &str,
    ) -> Result<(), Error> {
        let expression = format!(
            "(() => {{ try {{ document.querySelector({}); }} catch (error) {{ return false; }} {action}; return true; }})()",
            json!(selector)
        );

        if self.evaluate_utility(&expression).await? == json!(true) {
            return Ok(());
        }

        Err(Error::InvalidArgument {
            field: field.to_owned(),
            message: format!("`{selector}` is not a valid CSS selector"),
        })
    }

    pub async fn set_extra_headers(&self, headers: &BTreeMap<String, String>) -> Result<(), Error> {
        if let Session::Camoufox(session) = &self.session {
            return session.set_extra_headers(headers).await;
        }
        self.page_call("Network.setExtraHTTPHeaders", json!({ "headers": headers }))
            .await?;

        Ok(())
    }
}

fn add_style(css: &str) -> String {
    format!(
        "{{ const style = document.createElement('style'); style.textContent = {}; (document.head ?? document.documentElement).append(style); }}",
        json!(css)
    )
}
