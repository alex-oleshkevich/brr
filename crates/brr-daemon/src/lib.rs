pub mod app;
pub mod http;
pub mod mcp;
pub mod ops;
pub mod remote;
pub mod services;

mod lock;
mod run;
pub mod wire;

use std::fmt::Display;

use brr_core::Error;

pub use app::{App, Boot, CallClient, Ctx};
pub use run::{run, serve};

pub(crate) fn internal(e: impl Display) -> Error {
    Error::Internal {
        message: e.to_string(),
    }
}
