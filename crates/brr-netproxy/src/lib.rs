mod forward;
mod head;
mod health;
mod upstream;

// The shared test support is also a module of this crate and names it by its crate name.
#[cfg(test)]
extern crate self as brr_netproxy;

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod support;

use std::time::Duration;

use brr_core::{ProxyScheme, Secret};

pub use forward::{Forwarder, Listen, Meter, Route};
pub use health::{Health, check};

pub const HEAD_MAX_BYTES: usize = 64 * 1024;
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub struct Upstream {
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<Secret>,
}
