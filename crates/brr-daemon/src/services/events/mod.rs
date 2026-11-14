use std::sync::Arc;

use brr_core::{ClientId, Error, Event, EventData, Subject, now_ms};
use brr_store::{Store, event};
use serde_json::Value;
use tokio::sync::{Mutex, broadcast};

use crate::internal;

#[cfg(test)]
mod tests;

pub const SSE_BUFFER: usize = 1024;

// Transient events never reach the table, so the store's high-water is reserved this far ahead;
// a restart resumes past every seq a client can have seen.
const SEQ_RESERVE_BLOCK: i64 = 1024;

const KIND_SEPARATOR: char = ',';
const SUBJECT_SEPARATOR: char = ':';

tokio::task_local! {
    // The Client on whose behalf the running call publishes; unset for daemon-initiated work.
    static ACTOR: ClientId;
}

// Runs `work` with every event it publishes attributed to `client_id`.
pub async fn as_actor<T>(client_id: ClientId, work: impl Future<Output = T>) -> T {
    ACTOR.scope(client_id, work).await
}

// For work handed to `tokio::spawn`, which does not inherit the call's actor.
pub fn keep_actor<T>(work: impl Future<Output = T>) -> impl Future<Output = T> {
    let actor = ACTOR.try_with(Clone::clone).ok();

    async move {
        match actor {
            Some(client_id) => as_actor(client_id, work).await,
            None => work.await,
        }
    }
}

#[derive(Debug)]
pub struct Published {
    pub event: Event,
    pub json: String,
}

pub struct EventBus {
    tx: broadcast::Sender<Arc<Published>>,
    // Seq assignment, the insert and the broadcast share this lock, so subscribers see seq order.
    seq: Mutex<SeqState>,
    boot_seq: i64,
    store: Store,
}

struct SeqState {
    last: i64,
    reserved: i64,
}

impl EventBus {
    pub async fn new(store: Store) -> Result<EventBus, Error> {
        let last_seq = store.run(|c| event::max_seq(c)).await?;

        Ok(EventBus {
            tx: broadcast::Sender::new(SSE_BUFFER),
            seq: Mutex::new(SeqState {
                last: last_seq,
                reserved: last_seq,
            }),
            boot_seq: last_seq,
            store,
        })
    }

    pub async fn publish(&self, data: EventData) -> Result<Arc<Published>, Error> {
        let mut state = self.seq.lock().await;
        let seq = state.last + 1;
        // A transient event still takes a seq; the gap in the table is expected.
        state.last = seq;

        let mut event = Event::new(seq, now_ms(), &data);
        event.actor_client_id = ACTOR.try_with(Clone::clone).ok();

        if !data.transient() {
            let row = event.clone();
            self.store.run(move |c| event::insert(c, &row)).await?;
            state.reserved = state.reserved.max(seq);
        } else if seq > state.reserved {
            let reserve_to = seq + SEQ_RESERVE_BLOCK;
            self.store
                .run(move |c| event::reserve_seq(c, reserve_to))
                .await?;
            state.reserved = reserve_to;
        }

        let json = serde_json::to_string(&event).map_err(internal)?;
        let published = Arc::new(Published { event, json });
        // No receivers is fine.
        let _ = self.tx.send(Arc::clone(&published));

        Ok(published)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Published>> {
        self.tx.subscribe()
    }

    pub async fn last_seq(&self) -> i64 {
        self.seq.lock().await.last
    }

    pub fn boot_seq(&self) -> i64 {
        self.boot_seq
    }
}

pub fn split_kinds(kinds: Option<&str>) -> Vec<String> {
    kinds
        .unwrap_or_default()
        .split(KIND_SEPARATOR)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn parse_subject(text: &str) -> Result<Subject, Error> {
    let invalid = |message: String| Error::InvalidArgument {
        field: "subject".to_string(),
        message,
    };
    let (kind, id) = text
        .split_once(SUBJECT_SEPARATOR)
        .ok_or_else(|| invalid("expected <type>:<id>, for example instance:01J...".to_string()))?;
    let kind = serde_json::from_value(Value::String(kind.to_string()))
        .map_err(|_| invalid(format!("unknown subject type `{kind}`")))?;

    Ok(Subject {
        kind,
        id: id.to_string(),
    })
}
