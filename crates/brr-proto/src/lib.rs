mod calls;
mod mux;
mod pipe;
mod waiters;

pub use calls::RemoteError;
pub use mux::Mux;
pub use pipe::{Spawned, spawn};
pub use waiters::{Matcher, SessionId, Waiters};

pub const MAX_MESSAGE_BYTES: usize = 256 * 1024 * 1024;
// Method of the synthetic event that carries the answer to `Mux::forward`.
pub const FORWARDED: &str = "brr.forwarded";
pub const EVENT_QUEUE: usize = 1024;
// Callers that must not retry a dead browser match on this text; brr_core::Error has no variant.
pub const CONNECTION_CLOSED: &str = "browser connection closed";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("connection closed")]
    Closed,
    #[error("{message} ({code})")]
    Remote { code: i64, message: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct Incoming {
    pub method: String,
    pub params: serde_json::Value,
    pub session_id: Option<String>,
}
