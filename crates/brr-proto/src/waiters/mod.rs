use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::Incoming;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

pub type Matcher = Box<dyn Fn(&Value) -> bool + Send + Sync>;

struct Waiter {
    session_id: Option<SessionId>,
    method: &'static str,
    matches: Matcher,
    reply: oneshot::Sender<Value>,
}

// The one-shot event waiters of a connection. The table is None once the stream has ended, so a
// late waiter fails instead of waiting forever.
#[derive(Clone)]
pub struct Waiters(Arc<Mutex<Option<Vec<Waiter>>>>);

impl Default for Waiters {
    fn default() -> Waiters {
        Waiters(Arc::new(Mutex::new(Some(Vec::new()))))
    }
}

impl Waiters {
    pub fn wait_for(
        &self,
        session_id: Option<&SessionId>,
        method: &'static str,
        matches: Matcher,
    ) -> oneshot::Receiver<Value> {
        let (reply, receiver) = oneshot::channel();
        if let Some(waiters) = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            waiters.push(Waiter {
                session_id: session_id.cloned(),
                method,
                matches,
                reply,
            });
        }

        receiver
    }

    pub fn pending(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or(0, Vec::len)
    }

    // Answers the waiters the frame satisfies; waiters nobody listens to anymore are dropped.
    pub fn resolve(&self, frame: &Incoming) {
        let mut guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(waiters) = guard.as_mut() else {
            return;
        };
        let mut index = 0;

        while index < waiters.len() {
            let waiter = &waiters[index];
            let hit = waiter.method == frame.method
                && waiter.session_id.as_ref().map(|session| &session.0)
                    == frame.session_id.as_ref();

            if waiter.reply.is_closed() {
                waiters.swap_remove(index);
            } else if hit && (waiter.matches)(&frame.params) {
                let _ = waiters.swap_remove(index).reply.send(frame.params.clone());
            } else {
                index += 1;
            }
        }
    }

    // Dropping the senders wakes every waiter with an error instead of leaving it to time out.
    pub fn close(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
    }
}
