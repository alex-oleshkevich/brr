use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
};
use brr_core::{ClientId, Error, Event, Order, Page, Subject};
use brr_store::event::{self, Query as EventQuery};
use futures::Stream;
use serde::Deserialize;
use tokio::sync::{
    broadcast::{self, error::RecvError},
    mpsc::{self, error::SendTimeoutError},
};

use super::ops::{ApiError, client_header};
use crate::{
    app::App,
    services::{
        clients::Hold,
        events::{Published, parse_subject, split_kinds},
    },
    wire::LAGGED,
};

const LAST_EVENT_ID: &str = "last-event-id";
const CONNECTED: &str = "connected";
const KEEP_ALIVE: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const REPLAY_PAGE: u32 = 500;
const OUTBOX: usize = 64;

type Frame = Result<SseEvent, Infallible>;

#[derive(Deserialize)]
pub struct StreamQuery {
    kinds: Option<String>,
    subject: Option<String>,
    since: Option<String>,
}

struct Filter {
    kinds: Vec<String>,
    subject: Option<Subject>,
}

impl Filter {
    fn parse(query: &StreamQuery) -> Result<Filter, Error> {
        let kinds = split_kinds(query.kinds.as_deref());
        let subject = query.subject.as_deref().map(parse_subject).transpose()?;

        Ok(Filter { kinds, subject })
    }

    fn matches(&self, event: &Event) -> bool {
        (self.kinds.is_empty() || self.kinds.contains(&event.kind))
            && self.subject.as_ref().is_none_or(|s| *s == event.subject)
    }
}

pub async fn stream(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Frame>>, ApiError> {
    let filter = Filter::parse(&query)?;
    let since = query
        .since
        .as_deref()
        .map(|since| {
            since.parse().map_err(|_| Error::InvalidArgument {
                field: "since".to_string(),
                message: "expected an event sequence number".to_string(),
            })
        })
        .transpose()?;
    // The Hold keeps a UI Client alive for as long as its stream is open.
    let hold = match client_header(&headers)? {
        Some(header) => Some(app.clients.hold(&app, &ClientId::parse(header)?).await?),
        None => None,
    };

    // Subscribing before the replay leaves no gap; the pump drops duplicates by seq.
    let live = app.events.subscribe();
    let resume = headers
        .get(LAST_EVENT_ID)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .or(since);
    let after = start_after(resume, app.events.last_seq().await);

    let (tx, rx) = mpsc::channel(OUTBOX);
    // Without a first frame the body stays empty until an event or the keep-alive, and proxies hang.
    let _ = tx.try_send(Ok(SseEvent::default().comment(CONNECTED)));
    tokio::spawn(pump(app, hold, filter, live, after, tx));

    let frames = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|frame| (frame, rx))
    });

    Ok(Sse::new(frames).keep_alive(KeepAlive::new().interval(KEEP_ALIVE)))
}

async fn pump(
    app: Arc<App>,
    _hold: Option<Hold>,
    filter: Filter,
    mut live: broadcast::Receiver<Arc<Published>>,
    mut last: i64,
    tx: mpsc::Sender<Frame>,
) {
    match replay(&app, &filter, &mut last, &tx).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            tracing::warn!(error = format!("{error:#}"), "event replay failed");

            return;
        }
    }

    loop {
        let received = tokio::select! {
            () = tx.closed() => return,
            received = live.recv() => received,
        };

        match received {
            Ok(published) => {
                let event = &published.event;
                if event.seq <= last || !filter.matches(event) {
                    continue;
                }

                last = event.seq;
                if !deliver(&tx, frame(event.seq, &event.kind, &published.json)).await {
                    return;
                }
            }
            // The client reconnects with Last-Event-ID and replays what it missed.
            Err(RecvError::Lagged(_)) => {
                deliver(&tx, SseEvent::default().event(LAGGED).data(LAGGED)).await;

                return;
            }
            Err(RecvError::Closed) => return,
        }
    }
}

// A reader that stops draining fills the outbox; waiting forever would keep its Hold (and the
// Client's Leases) alive, so it is disconnected after WRITE_TIMEOUT. False means stop pumping.
async fn deliver(tx: &mpsc::Sender<Frame>, frame: SseEvent) -> bool {
    match tx.send_timeout(Ok(frame), WRITE_TIMEOUT).await {
        Ok(()) => true,
        Err(SendTimeoutError::Timeout(_)) => {
            tracing::warn!("event stream reader stalled; disconnecting");

            false
        }
        Err(SendTimeoutError::Closed(_)) => false,
    }
}

// False when the reader left or stalled and the pump must stop.
async fn replay(
    app: &App,
    filter: &Filter,
    last: &mut i64,
    tx: &mpsc::Sender<Frame>,
) -> Result<bool, Error> {
    loop {
        let query = EventQuery {
            since: Some(*last),
            kinds: filter.kinds.clone(),
            subject: filter.subject.clone(),
            instance_id: None,
            from_ts: None,
            to_ts: None,
            order: Order::Asc,
            page: Page::new(Some(REPLAY_PAGE), None),
        };
        let (events, _) = app.store.run(move |c| event::list(c, &query)).await?;
        let last_page = events.len() < REPLAY_PAGE as usize;

        for event in events {
            let json = serde_json::to_string(&event).map_err(crate::internal)?;
            *last = event.seq;

            if !deliver(tx, frame(event.seq, &event.kind, &json)).await {
                return Ok(false);
            }
        }

        if last_page {
            return Ok(true);
        }
    }
}

fn frame(seq: i64, kind: &str, json: &str) -> SseEvent {
    SseEvent::default()
        .id(seq.to_string())
        .event(kind)
        .data(json)
}

// A cursor above anything this daemon issued (a restored database, a made-up number) would skip
// every new event, so it counts as "now".
fn start_after(resume: Option<i64>, last_seq: i64) -> i64 {
    resume.map_or(last_seq, |seq| seq.min(last_seq))
}

#[cfg(test)]
mod tests;
