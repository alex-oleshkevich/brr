use brr_core::{Cookie, Error};
use serde_json::{Value, json};

use super::{Camoufox, PageSession};
use crate::page::{cookie_params, parse_cookies};

// Juggler's cookie shape is Playwright's, which is also the storageState shape brr uses, and
// the one CDP's cookies have.
fn parse(mut result: Value) -> Result<Vec<Cookie>, Error> {
    parse_cookies(&mut result)
}

fn stored(cookies: &[Cookie]) -> Value {
    json!({ "cookies": cookie_params(cookies) })
}

impl Camoufox {
    pub(crate) async fn cookies(&self) -> Result<Vec<Cookie>, Error> {
        parse(
            self.inner
                .juggler
                .call(None, "Browser.getCookies", json!({}))
                .await?,
        )
    }

    pub(crate) async fn set_cookies(&self, cookies: &[Cookie]) -> Result<u32, Error> {
        self.inner
            .juggler
            .call(None, "Browser.setCookies", stored(cookies))
            .await?;

        Ok(u32::try_from(cookies.len()).unwrap_or(u32::MAX))
    }
}

impl PageSession {
    pub(crate) async fn cookies(&self) -> Result<Vec<Cookie>, Error> {
        parse(self.browser_call("Browser.getCookies", json!({})).await?)
    }

    pub(crate) async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), Error> {
        self.browser_call("Browser.setCookies", stored(cookies))
            .await?;

        Ok(())
    }

    pub(crate) async fn clear_cookies(&self) -> Result<(), Error> {
        self.browser_call("Browser.clearCookies", json!({})).await?;

        Ok(())
    }
}
