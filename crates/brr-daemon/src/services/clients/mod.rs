use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::Duration,
};

use brr_core::{
    Client, ClientEndReason, ClientId, ClientKind, Error, EventData, ReleaseReason, TabId, now_ms,
};
use brr_store::{Store, client, lease};
use tokio::{
    sync::{Mutex, Notify},
    time,
};

use crate::{
    app::{App, CallClient},
    internal,
    services::events::EventBus,
};

#[cfg(test)]
mod tests;

pub const UI_HEARTBEAT: Duration = Duration::from_secs(120);

const LABEL_MAX_LEN: usize = 64;
const IMPLICIT_LABEL: &str = "implicit";
const SECS_PER_MIN: u64 = 60;

struct Entry {
    client: Client,
    holds: u32,
    // Calls in flight: the idle timer does not run through a call that is still working.
    calls: u32,
    touched: Arc<Notify>,
}

#[derive(Default)]
pub struct Clients {
    entries: Mutex<HashMap<ClientId, Entry>>,
}

pub struct ImplicitClient {
    pub client: CallClient,
    app: Arc<App>,
    finished: bool,
}

pub struct Hold {
    app: Arc<App>,
    client_id: ClientId,
}

// Held for the length of a call; an implicit Client has no entry and nothing to suspend.
pub struct CallGuard {
    app: Arc<App>,
    client_id: ClientId,
}

impl Clients {
    pub async fn create(
        &self,
        app: &Arc<App>,
        kind: ClientKind,
        label: String,
    ) -> Result<Client, Error> {
        if label.chars().count() > LABEL_MAX_LEN {
            return Err(Error::InvalidArgument {
                field: "label".to_string(),
                message: format!("at most {LABEL_MAX_LEN} characters"),
            });
        }

        let mut entries = self.entries.lock().await;

        register(app, &mut entries, ClientId::new(), kind, label).await
    }

    pub async fn resolve(
        &self,
        _app: &Arc<App>,
        client_id: &ClientId,
    ) -> Result<CallClient, Error> {
        let entries = self.entries.lock().await;
        let entry = entries
            .get(client_id)
            .ok_or_else(|| not_active(client_id))?;
        entry.touched.notify_one();

        Ok(call_client(&entry.client))
    }

    pub async fn resolve_rest(&self, app: &Arc<App>, header: &str) -> Result<CallClient, Error> {
        let client_id = ClientId::parse(header)?;
        let mut entries = self.entries.lock().await;

        if let Some(entry) = entries.get(&client_id) {
            entry.touched.notify_one();

            return Ok(call_client(&entry.client));
        }

        // Only a REST id may be made again by its header; any other kind needs client_create.
        match stored(&app.store, &client_id).await {
            Ok(row) if row.kind != ClientKind::Rest => return Err(not_active(&client_id)),
            Ok(_) | Err(Error::NotFound { .. }) => {}
            Err(error) => return Err(error),
        }

        let label = client_id.to_string();
        let client = register(app, &mut entries, client_id, ClientKind::Rest, label).await?;

        Ok(call_client(&client))
    }

    pub fn implicit(&self, app: &Arc<App>, kind: ClientKind) -> ImplicitClient {
        ImplicitClient {
            client: CallClient {
                client_id: ClientId::new(),
                kind,
                label: IMPLICIT_LABEL.to_string(),
                implicit: true,
            },
            app: Arc::clone(app),
            finished: false,
        }
    }

    pub async fn in_call(&self, app: &Arc<App>, client_id: &ClientId) -> CallGuard {
        if let Some(entry) = self.entries.lock().await.get_mut(client_id) {
            entry.calls += 1;
            // The timer that is running stops waiting for a Client that is busy.
            entry.touched.notify_one();
        }

        CallGuard {
            app: Arc::clone(app),
            client_id: client_id.clone(),
        }
    }

    pub async fn hold(&self, app: &Arc<App>, client_id: &ClientId) -> Result<Hold, Error> {
        let mut entries = self.entries.lock().await;
        let entry = entries
            .get_mut(client_id)
            .ok_or_else(|| not_active(client_id))?;
        entry.holds += 1;

        Ok(Hold {
            app: Arc::clone(app),
            client_id: client_id.clone(),
        })
    }

    pub async fn end(
        &self,
        app: &Arc<App>,
        client_id: &ClientId,
        reason: ClientEndReason,
    ) -> Result<Client, Error> {
        // The Leases are released after the Client is gone, so the work runs in its own task:
        // a cancelled caller must not leave an ended Client holding Leases.
        self.end_spawned(app, client_id, reason, false).await
    }

    async fn end_spawned(
        &self,
        app: &Arc<App>,
        client_id: &ClientId,
        reason: ClientEndReason,
        only_idle: bool,
    ) -> Result<Client, Error> {
        let app = Arc::clone(app);
        let client_id = client_id.clone();

        tokio::spawn(async move {
            app.clients
                .end_now(&app, &client_id, reason, only_idle)
                .await
        })
        .await
        .map_err(internal)?
    }

    // With `only_idle`, a Client that gained a Hold since its deadline passed is left alone.
    async fn end_now(
        &self,
        app: &Arc<App>,
        client_id: &ClientId,
        reason: ClientEndReason,
        only_idle: bool,
    ) -> Result<Client, Error> {
        let mut entries = self.entries.lock().await;
        if only_idle
            && entries
                .get(client_id)
                .is_some_and(|e| e.holds > 0 || e.calls > 0)
        {
            drop(entries);

            return stored(&app.store, client_id).await;
        }

        let Some(entry) = entries.remove(client_id) else {
            drop(entries);

            return stored(&app.store, client_id).await;
        };

        let now = now_ms();
        let id = client_id.clone();
        if let Err(error) = app.store.run(move |c| client::end(c, &id, now)).await {
            entries.insert(client_id.clone(), entry);

            return Err(error);
        }
        drop(entries);

        // Wakes the expiry task so it sees the entry gone and stops.
        entry.touched.notify_one();
        let mut client = entry.client;
        client.ended_at = Some(now);

        // A Lease acquired before the entry went away is caught here; later acquisitions see the
        // Client inactive.
        app.leases
            .release_client(app, client_id, ReleaseReason::ClientEnded)
            .await?;
        app.events
            .publish(EventData::ClientEnded {
                client_id: client_id.clone(),
                reason,
            })
            .await?;

        Ok(client)
    }

    pub(crate) async fn is_active(&self, client_id: &ClientId) -> bool {
        self.entries.lock().await.contains_key(client_id)
    }

    pub async fn set_current_tab(
        &self,
        app: &Arc<App>,
        client_id: &ClientId,
        tab_id: Option<&TabId>,
    ) -> Result<(), Error> {
        let mut entries = self.entries.lock().await;

        // An implicit Client has no entry: its current Tab lives only as long as its call.
        let Some(entry) = entries.get_mut(client_id) else {
            return Ok(());
        };

        // Memory first: a cancelled call leaves the write that already reached the store in step.
        let previous = entry.client.current_tab_id;
        entry.client.current_tab_id = tab_id.cloned();

        let id = client_id.clone();
        let tab = tab_id.cloned();
        let saved = app
            .store
            .run(move |c| client::set_current_tab(c, &id, tab.as_ref()))
            .await;
        if saved.is_err() {
            entry.client.current_tab_id = previous;
        }

        saved
    }

    pub async fn current_tab(&self, client_id: &ClientId) -> Option<TabId> {
        let entries = self.entries.lock().await;

        entries.get(client_id)?.client.current_tab_id
    }

    // None: the Client is gone.
    async fn busy(&self, client_id: &ClientId) -> Option<bool> {
        let entries = self.entries.lock().await;

        entries
            .get(client_id)
            .map(|entry| entry.holds > 0 || entry.calls > 0)
    }

    async fn end_call(&self, client_id: &ClientId) {
        let mut entries = self.entries.lock().await;
        let Some(entry) = entries.get_mut(client_id) else {
            return;
        };

        entry.calls = entry.calls.saturating_sub(1);
        if entry.calls == 0 {
            // The deadline restarts from the end of the call.
            entry.touched.notify_one();
        }
    }

    async fn release_hold(&self, app: &Arc<App>, client_id: &ClientId) -> Result<(), Error> {
        let mut entries = self.entries.lock().await;
        let Some(entry) = entries.get_mut(client_id) else {
            return Ok(());
        };

        entry.holds = entry.holds.saturating_sub(1);
        if entry.holds > 0 {
            return Ok(());
        }

        // The deadline restarts for Clients that outlive their connection.
        entry.touched.notify_one();
        let ends_now = matches!(entry.client.kind, ClientKind::Cli | ClientKind::Mcp);
        drop(entries);

        if ends_now {
            self.end(app, client_id, ClientEndReason::Disconnected)
                .await?;
        }

        Ok(())
    }
}

impl ImplicitClient {
    pub async fn finish(mut self) {
        release(&self.app, &self.client.client_id).await;
        // After the release: a cancelled finish still lets Drop release.
        self.finished = true;
    }
}

impl Drop for ImplicitClient {
    fn drop(&mut self) {
        if self.finished {
            return;
        }

        let app = Arc::clone(&self.app);
        let client_id = self.client.client_id.clone();
        tokio::spawn(async move { release(&app, &client_id).await });
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        let app = Arc::clone(&self.app);
        let client_id = self.client_id.clone();

        tokio::spawn(async move { app.clients.end_call(&client_id).await });
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let app = Arc::clone(&self.app);
        let client_id = self.client_id.clone();

        tokio::spawn(async move {
            if let Err(error) = app.clients.release_hold(&app, &client_id).await {
                tracing::warn!(%client_id, %error, "client hold release failed");
            }
        });
    }
}

// An end at startup: nothing survives a daemon restart, so every active row is closed.
pub(crate) async fn reconcile(store: &Store, events: &EventBus) -> Result<(), Error> {
    let now = now_ms();
    let active = store.run(|c| client::active(c)).await?;

    for stale in active {
        let id = stale.id.clone();
        store.run(move |c| client::end(c, &id, now)).await?;
        events
            .publish(EventData::ClientEnded {
                client_id: stale.id,
                reason: ClientEndReason::DaemonRestart,
            })
            .await?;
    }

    store
        .run(move |c| lease::release_all(c, ReleaseReason::DaemonRestart, now))
        .await?;

    Ok(())
}

async fn release(app: &Arc<App>, client_id: &ClientId) {
    if let Err(error) = app
        .leases
        .release_client(app, client_id, ReleaseReason::ClientEnded)
        .await
    {
        tracing::warn!(%client_id, %error, "implicit client lease release failed");
    }
}

async fn stored(store: &Store, client_id: &ClientId) -> Result<Client, Error> {
    let id = client_id.clone();

    store.run(move |c| client::get(c, &id)).await
}

pub(crate) fn not_active(client_id: &ClientId) -> Error {
    Error::NotFound {
        what: format!("client {client_id} (call client_create again)"),
    }
}

fn call_client(client: &Client) -> CallClient {
    CallClient {
        client_id: client.id.clone(),
        kind: client.kind,
        label: client.label.clone(),
        implicit: false,
    }
}

// Registers the Client, persists and announces it, and starts its expiry task, all under the caller's lock.
async fn register(
    app: &Arc<App>,
    entries: &mut HashMap<ClientId, Entry>,
    client_id: ClientId,
    kind: ClientKind,
    label: String,
) -> Result<Client, Error> {
    let client = Client {
        id: client_id.clone(),
        kind,
        label: label.clone(),
        current_tab_id: None,
        created_at: now_ms(),
        ended_at: None,
    };

    let touched = Arc::new(Notify::new());
    entries.insert(
        client_id.clone(),
        Entry {
            client: client.clone(),
            holds: 0,
            calls: 0,
            touched: Arc::clone(&touched),
        },
    );

    // Spawned before any await so a cancelled caller still leaves the Client with its timer.
    if idle_limit(app, kind).is_some() {
        tokio::spawn(expire(
            Arc::downgrade(app),
            client_id.clone(),
            kind,
            touched,
        ));
    }

    let row = client.clone();
    if let Err(error) = app.store.run(move |c| client::upsert(c, &row)).await {
        entries.remove(&client_id);

        return Err(error);
    }

    app.events
        .publish(EventData::ClientCreated {
            client_id: client_id.clone(),
            kind,
            label,
        })
        .await?;

    Ok(client)
}

// None: the Client never expires by idling (a connect Client lives as long as its tunnel).
fn idle_limit(app: &App, kind: ClientKind) -> Option<Duration> {
    let settings = app.settings();
    let minutes = match kind {
        ClientKind::Ui => return Some(UI_HEARTBEAT),
        ClientKind::Mcp => settings.client.mcp_http_idle_min,
        ClientKind::Rest | ClientKind::Cli => settings.client.rest_idle_min,
        ClientKind::Connect => return None,
    };

    Some(Duration::from_secs(u64::from(minutes) * SECS_PER_MIN))
}

// Weak, like every background task: a strong reference would keep the daemon lock until the
// Client expires.
async fn expire(app: Weak<App>, client_id: ClientId, kind: ClientKind, touched: Arc<Notify>) {
    loop {
        let Some(strong) = app.upgrade() else {
            return;
        };
        let Some(idle) = idle_limit(&strong, kind) else {
            return;
        };

        match strong.clients.busy(&client_id).await {
            None => return,
            // The deadline is suspended while a connection or a call is open; its end notifies.
            Some(true) => {
                drop(strong);
                touched.notified().await;

                continue;
            }
            Some(false) => drop(strong),
        }

        tokio::select! {
            () = time::sleep(idle) => {
                let Some(strong) = app.upgrade() else {
                    return;
                };
                let ended = strong
                    .clients
                    .end_spawned(&strong, &client_id, ClientEndReason::Idle, true)
                    .await;
                if let Err(error) = ended {
                    tracing::warn!(%client_id, %error, "idle client end failed");

                    return;
                }
            }
            () = touched.notified() => {}
        }
    }
}
