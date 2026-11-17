use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::Duration,
};

use brr_bitwarden::{self as bitwarden, Credentials, ItemSummary, Login, Vault};
use brr_core::{
    BitwardenItem, BitwardenStatus, Credential, CredentialId, CredentialSelectors,
    CredentialSource, Error, EventData, Secret, now_ms, site_origin,
};
use brr_store::{credential, secret};
use serde_json::Value;
use tokio::{
    select,
    sync::{
        Mutex, Notify,
        broadcast::{Receiver, error::RecvError},
    },
    time::Instant,
};
use tracing::warn;
use ulid::Ulid;

use crate::{
    app::App,
    internal,
    services::{
        SETTINGS_CHANGED,
        credentials::{self, resolve_profiles},
        events::Published,
        until,
    },
};

#[cfg(test)]
mod tests;

pub const CLIENT_ID: &str = "bitwarden.client_id";
pub const CLIENT_SECRET: &str = "bitwarden.client_secret";
pub const MASTER_PASSWORD: &str = "bitwarden.master_password";
// Not a secret, but kept beside them: Bitwarden lists each new device id as a new login.
const DEVICE_ID: &str = "bitwarden.device_id";
const SERVER_URL_SETTING: &str = "bitwarden.server_url";
const MS_PER_MIN: i64 = 60_000;
const NO_LOGIN_URI: &str = "the Bitwarden item has no http:// or https:// URI to log in at";

#[derive(Default)]
pub struct Bitwarden {
    state: RwLock<State>,
    // One open at a time, so a slow sync cannot bring back a vault a disconnect just dropped.
    opening: Mutex<()>,
    // Wakes the sync timer when a connection starts or ends.
    wake: Arc<Notify>,
}

#[derive(Default)]
struct State {
    vault: Option<Vault>,
    items: Vec<ItemSummary>,
    synced_at: Option<i64>,
    attempted_at: Option<i64>,
    error: Option<String>,
}

impl Bitwarden {
    // Held weakly: a strong reference would keep the daemon lock after the last owner drops the App.
    pub fn start(app: &Arc<App>) {
        let weak = Arc::downgrade(app);
        let wake = Arc::clone(&app.bitwarden.wake);
        let mut events = app.events.subscribe();

        tokio::spawn(async move {
            // The first open runs here, so a slow server never holds up the listener.
            let mut due = Some(Instant::now());

            loop {
                select! {
                    () = until(due) => {
                        let Some(app) = weak.upgrade() else { return };
                        if let Err(error) = reopen(&app).await {
                            warn!(%error, "bitwarden sync failed");
                        }
                    }
                    () = wake.notified() => {}
                    alive = settings_changed(&mut events) => {
                        if !alive {
                            return;
                        }
                    }
                }

                let Some(app) = weak.upgrade() else { return };
                due = next_sync(&app).await;
            }
        });
    }

    // For display only: the username and TOTP flag the last sync holds for a linked item.
    pub fn summary(&self, item_id: &str) -> (String, bool) {
        self.read()
            .items
            .iter()
            .find(|item| item.id == item_id)
            .map_or_else(Default::default, |item| {
                (item.username.clone().unwrap_or_default(), item.has_totp)
            })
    }

    fn read(&self) -> RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn opened(&self, vault: Vault) -> u32 {
        let items = vault.items();
        let count = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let now = now_ms();
        *self.write() = State {
            vault: Some(vault),
            items,
            synced_at: Some(now),
            attempted_at: Some(now),
            error: None,
        };

        count
    }

    // The last good vault stays: logins keep working through a server outage.
    fn failed(&self, error: &Error) {
        let mut state = self.write();
        state.attempted_at = Some(now_ms());
        state.error = Some(error.to_string());
    }
}

pub async fn status(app: &App) -> Result<BitwardenStatus, Error> {
    let connected = connected(app).await?;
    let state = app.bitwarden.read();

    Ok(BitwardenStatus {
        connected,
        unlocked: state.vault.is_some(),
        email: state.vault.as_ref().map(|vault| vault.email().to_owned()),
        items: u32::try_from(state.items.len()).unwrap_or(u32::MAX),
        synced_at: state.synced_at,
        error: state.error.clone(),
    })
}

// Nothing is stored unless the vault opens with the given values.
pub async fn connect(
    app: &Arc<App>,
    server_url: &str,
    creds: Credentials,
) -> Result<BitwardenStatus, Error> {
    let server_url = server_url.trim().trim_end_matches('/').to_owned();
    let mut checked = (*app.settings()).clone();
    checked.bitwarden.server_url.clone_from(&server_url);
    checked.validate().map_err(|error| match error {
        Error::InvalidArgument { field, message } if field == SERVER_URL_SETTING => {
            Error::InvalidArgument {
                field: "server_url".to_owned(),
                message,
            }
        }
        other => other,
    })?;

    let opening = app.bitwarden.opening.lock().await;
    let device_id = match app.secrets.get(&app.store, DEVICE_ID).await? {
        Some(device_id) => device_id,
        None => Secret::new(Ulid::generate().to_string()),
    };
    let vault = Vault::open(&app.http, &server_url, device_id.expose(), &creds)
        .await
        .map_err(|error| to_error(error, &server_url))?;

    if app.settings().bitwarden.server_url != server_url {
        let changes = BTreeMap::from([(SERVER_URL_SETTING.to_owned(), Value::String(server_url))]);
        app.set_settings(changes).await?;
    }
    let client_id = Secret::new(creds.client_id.as_str());
    for (name, value) in [
        (DEVICE_ID, &device_id),
        (CLIENT_ID, &client_id),
        (CLIENT_SECRET, &creds.client_secret),
        (MASTER_PASSWORD, &creds.master_password),
    ] {
        app.secrets.set(&app.store, name, value).await?;
    }

    let items = app.bitwarden.opened(vault);
    drop(opening);
    app.bitwarden.wake.notify_one();
    app.events
        .publish(EventData::BitwardenSynced { items })
        .await?;

    status(app).await
}

pub async fn sync(app: &App) -> Result<BitwardenStatus, Error> {
    if !reopen(app).await? {
        return Err(Error::BitwardenLocked);
    }

    status(app).await
}

// Linked credentials stay and fail with BitwardenLocked until the next connect.
pub async fn disconnect(app: &App) -> Result<BitwardenStatus, Error> {
    let opening = app.bitwarden.opening.lock().await;
    for name in [CLIENT_ID, CLIENT_SECRET, MASTER_PASSWORD] {
        app.secrets.delete(&app.store, name).await?;
    }
    *app.bitwarden.write() = State::default();
    drop(opening);
    app.bitwarden.wake.notify_one();

    status(app).await
}

pub async fn items(app: &App, query: Option<&str>) -> Result<Vec<BitwardenItem>, Error> {
    let linked: HashMap<String, CredentialId> = app
        .store
        .run(|c| {
            Ok(credential::all(c)?
                .into_iter()
                .filter(|row| row.source == CredentialSource::Bitwarden)
                .filter_map(|row| row.bitwarden_item_id.map(|item_id| (item_id, row.id)))
                .collect())
        })
        .await?;
    let query = query.unwrap_or_default().to_lowercase();
    let state = app.bitwarden.read();
    if state.vault.is_none() {
        return Err(Error::BitwardenLocked);
    }

    Ok(state
        .items
        .iter()
        .filter(|item| matches(item, &query))
        .map(|item| BitwardenItem {
            id: item.id.clone(),
            name: item.name.clone(),
            uris: item.uris.clone(),
            username: item.username.clone().unwrap_or_default(),
            has_totp: item.has_totp,
            linked: linked.get(&item.id).copied(),
        })
        .collect())
}

// An item already linked keeps its credential and gains the given Profiles.
pub async fn link(
    app: &Arc<App>,
    item_id: String,
    profiles: Vec<String>,
    selectors: Option<CredentialSelectors>,
) -> Result<Credential, Error> {
    // The item's first web URI is where its form lives, e.g. https://site/login, not the site root.
    let (site, login_url) = {
        let state = app.bitwarden.read();
        if state.vault.is_none() {
            return Err(Error::BitwardenLocked);
        }
        let item = state
            .items
            .iter()
            .find(|item| item.id == item_id)
            .ok_or_else(|| not_found(&item_id))?;

        item.uris
            .iter()
            .find_map(|uri| site_origin(uri).map(|site| (site, uri.clone())))
            .ok_or_else(|| Error::InvalidArgument {
                field: "item_id".to_owned(),
                message: NO_LOGIN_URI.to_owned(),
            })?
    };

    let credential_id = app
        .store
        .run(move |c| {
            let mut linked = resolve_profiles(c, &profiles)?;
            let tx = c.transaction().map_err(internal)?;

            let credential_id = match credential::find_by_bitwarden_item(&tx, &item_id)? {
                Some(mut row) => {
                    linked.extend(credential::profiles(&tx, &row.id)?);
                    linked.sort();
                    linked.dedup();
                    if let Some(selectors) = selectors {
                        row.selectors = selectors;
                        row.updated_at = now_ms();
                        credential::update(&tx, &row)?;
                    }

                    row.id
                }
                None => {
                    let now = now_ms();
                    let row = credential::CredentialRow {
                        id: CredentialId::new(),
                        site,
                        login_url: Some(login_url),
                        username_enc: None,
                        password_enc: None,
                        totp_enc: None,
                        notes_enc: None,
                        selectors: selectors.unwrap_or_default(),
                        created_at: now,
                        updated_at: now,
                        source: CredentialSource::Bitwarden,
                        bitwarden_item_id: Some(item_id),
                    };
                    credential::insert(&tx, &row)?;

                    row.id
                }
            };
            credential::set_profiles(&tx, &credential_id, &linked)?;
            tx.commit().map_err(internal)?;

            Ok(credential_id)
        })
        .await?;

    credentials::load(app, &credential_id).await
}

// Decrypts the item's secrets from the last sync; nothing is fetched.
pub fn login(app: &App, item_id: &str) -> Result<Login, Error> {
    let state = app.bitwarden.read();
    let vault = state.vault.as_ref().ok_or(Error::BitwardenLocked)?;

    vault
        .login(item_id)
        .map_err(|error| to_error(error, &app.settings().bitwarden.server_url))
}

// Ok(false): not connected, nothing to open.
async fn reopen(app: &App) -> Result<bool, Error> {
    let _opening = app.bitwarden.opening.lock().await;
    let Some((device_id, creds)) = stored(app).await? else {
        return Ok(false);
    };
    let server_url = app.settings().bitwarden.server_url.clone();

    match Vault::open(&app.http, &server_url, device_id.expose(), &creds).await {
        Ok(vault) => {
            let items = app.bitwarden.opened(vault);
            app.events
                .publish(EventData::BitwardenSynced { items })
                .await?;

            Ok(true)
        }
        Err(error) => {
            let error = to_error(error, &server_url);
            app.bitwarden.failed(&error);
            app.events
                .publish(EventData::BitwardenSyncFailed {
                    error: error.to_string(),
                })
                .await?;

            Err(error)
        }
    }
}

async fn stored(app: &App) -> Result<Option<(Secret, Credentials)>, Error> {
    let get = |name| app.secrets.get(&app.store, name);
    let (Some(device_id), Some(client_id), Some(client_secret), Some(master_password)) = (
        get(DEVICE_ID).await?,
        get(CLIENT_ID).await?,
        get(CLIENT_SECRET).await?,
        get(MASTER_PASSWORD).await?,
    ) else {
        return Ok(None);
    };

    Ok(Some((
        device_id,
        Credentials {
            client_id: client_id.expose().to_owned(),
            client_secret,
            master_password,
        },
    )))
}

async fn connected(app: &App) -> Result<bool, Error> {
    let found = app.store.run(|c| secret::get(c, CLIENT_ID)).await?;

    Ok(found.is_some())
}

async fn next_sync(app: &App) -> Option<Instant> {
    match connected(app).await {
        Ok(true) => {}
        Ok(false) => return None,
        Err(error) => {
            warn!(%error, "bitwarden sync timer: reading the connection failed");
            return None;
        }
    }

    let interval_ms = i64::from(app.settings().bitwarden.sync_interval_min) * MS_PER_MIN;
    let wait_ms = app
        .bitwarden
        .read()
        .attempted_at
        .map_or(0, |at| at + interval_ms - now_ms());

    Some(Instant::now() + Duration::from_millis(u64::try_from(wait_ms).unwrap_or(0)))
}

async fn settings_changed(events: &mut Receiver<Arc<Published>>) -> bool {
    loop {
        match events.recv().await {
            Ok(published) if published.event.kind == SETTINGS_CHANGED => return true,
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => return true,
            Err(RecvError::Closed) => return false,
        }
    }
}

fn matches(item: &ItemSummary, query: &str) -> bool {
    let has = |text: &str| text.to_lowercase().contains(query);

    has(&item.name)
        || item.uris.iter().any(|uri| has(uri))
        || item.username.as_deref().is_some_and(has)
}

fn not_found(item_id: &str) -> Error {
    Error::NotFound {
        what: format!("Bitwarden item {item_id}"),
    }
}

fn to_error(error: bitwarden::Error, server_url: &str) -> Error {
    let message = error.to_string();
    let field = match error {
        bitwarden::Error::InvalidServer(_) => "server_url",
        bitwarden::Error::Auth(_) => "client_secret",
        bitwarden::Error::WrongPassword => "master_password",
        bitwarden::Error::Unsupported(_) => "client_id",
        bitwarden::Error::NotFound(item_id) => return not_found(&item_id),
        bitwarden::Error::Network(_) | bitwarden::Error::Protocol(_) => {
            return Error::DependencyMissing {
                name: format!("the Bitwarden server {server_url}"),
                help: message,
            };
        }
    };

    Error::InvalidArgument {
        field: field.to_owned(),
        message,
    }
}
