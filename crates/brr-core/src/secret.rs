use std::fmt;

use schemars::JsonSchema;
use serde::Deserialize;
use zeroize::Zeroize;

#[derive(Deserialize, JsonSchema)]
#[serde(transparent)]
#[schemars(inline, extend("writeOnly" = true))]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret({} chars)", self.0.chars().count())
    }
}
