use std::time::Duration;

use brr_core::Error;
use serde_json::Value;

use crate::{CONNECTION_CLOSED, Mux};

// How a dialect words the browser's own error answer for `method`: the code and message it sent.
pub type RemoteError = fn(method: &str, code: i64, message: &str) -> Error;

impl Mux {
    // `call` bounded by `timeout`, with the failures every protocol shares worded one way.
    pub async fn call_within(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
        remote: RemoteError,
    ) -> Result<Value, Error> {
        match tokio::time::timeout(timeout, self.call(method, params, session_id)).await {
            Err(_) => Err(Error::Timeout {
                after_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            }),
            Ok(Ok(result)) => Ok(result),
            Ok(Err(crate::Error::Closed)) => Err(Error::Internal {
                message: CONNECTION_CLOSED.to_owned(),
            }),
            Ok(Err(crate::Error::Remote { code, message })) => Err(remote(method, code, &message)),
            Ok(Err(error)) => Err(Error::Internal {
                message: format!("{method}: {error}"),
            }),
        }
    }
}
