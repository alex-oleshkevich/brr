use std::{
    net::SocketAddr,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use axum::{
    Router,
    extract::{
        ConnectInfo, Extension, Query, State, WebSocketUpgrade,
        rejection::QueryRejection,
        ws::{CloseFrame, Message, WebSocket, rejection::WebSocketUpgradeRejection},
    },
    response::Response,
    routing::get,
};
use brr_cdp::{Downstream, Outbound};
use brr_core::{
    ClientEndReason, ClientId, ClientKind, Display, EndReason, Engine, Error, InstanceId,
    InstanceState, Profile, ProfileKind, ProxyChoice, ProxyId, TabId,
};
use brr_proto::MAX_MESSAGE_BYTES;
use brr_store::profile;
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::warn;

use super::ops::ApiError;
use crate::{
    app::App,
    ops::invalid,
    services::{
        clients::Hold,
        oneshot::{detached, launch_instance},
        profiles::resolve_build,
    },
    wire::{NORMAL_CLOSE, websocket_only},
};

const UNSUPPORTED: u16 = 1003;
const INTERNAL: u16 = 1011;
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/cdp", get(upgrade))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CdpQuery {
    profile: Option<String>,
    display: Option<Display>,
    proxy: Option<String>,
}

// Ends the connect Client, and with it the exclusive Lease, on every exit path; an Instance
// this connection launched goes with it.
struct Connection {
    app: Arc<App>,
    client_id: ClientId,
    launched: Option<InstanceId>,
    _hold: Hold,
}

impl Drop for Connection {
    fn drop(&mut self) {
        let app = Arc::clone(&self.app);
        let client_id = self.client_id.clone();
        let launched = self.launched;

        tokio::spawn(async move {
            if let Err(error) = app
                .clients
                .end(&app, &client_id, ClientEndReason::Disconnected)
                .await
            {
                warn!(%client_id, error = format!("{error:#}"), "cdp client not ended");
            }

            if let Some(instance_id) = launched
                && app.instances.view(&instance_id).is_some_and(|view| {
                    matches!(
                        view.instance.state,
                        InstanceState::Running | InstanceState::Idle
                    )
                })
                && let Err(error) = app
                    .instances
                    .stop(&app, &instance_id, EndReason::Stop)
                    .await
            {
                warn!(%instance_id, error = format!("{error:#}"), "cdp instance not stopped");
            }
        });
    }
}

// Everything that can fail is done before the upgrade, so a client sees a normal error
// response; afterwards only close codes remain.
async fn upgrade(
    State(app): State<Arc<App>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    query: Result<Query<CdpQuery>, QueryRejection>,
    socket: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let socket = socket.map_err(|_| websocket_only())?;
    let Query(query) = query.map_err(|rejection| Error::InvalidArgument {
        field: "query".to_string(),
        message: rejection.body_text(),
    })?;

    let profile = chromium_profile(&app, query.profile.clone()).await?;
    let label = peer.map_or_else(
        || "cdp".to_string(),
        |Extension(ConnectInfo(peer))| format!("cdp {peer}"),
    );
    let client = app.clients.create(&app, ClientKind::Connect, label).await?;
    let hold = app.clients.hold(&app, &client.id).await?;
    let mut connection = Connection {
        app: Arc::clone(&app),
        client_id: client.id,
        launched: None,
        _hold: hold,
    };

    let (instance_id, first_tab) = instance_for(&app, profile, query).await?;
    connection.launched = first_tab.map(|_| instance_id);
    let call = app.clients.resolve(&app, &connection.client_id).await?;
    app.leases
        .acquire_exclusive(&app, &call, &instance_id)
        .await?;
    if let Some(tab_id) = first_tab {
        app.leases
            .acquire_tab(&app, &call, &instance_id, &tab_id)
            .await?;
    }
    let (downstream, outbound) = app.instances.downstream(&instance_id).await?;

    // An upgrade that never completes never runs the callback below, which would hold the
    // Client, its Lease and the browser session for good: the watchdog drops them instead.
    let waiting = Arc::new(Mutex::new(Some((connection, downstream, outbound))));
    let watchdog = waiting.clone();
    tokio::spawn(async move {
        tokio::time::sleep(UPGRADE_TIMEOUT).await;
        drop(
            watchdog
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
    });

    Ok(socket
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let parts = waiting
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some((connection, downstream, outbound)) = parts {
                pump(socket, connection, instance_id, downstream, outbound).await;
            }
        }))
}

async fn chromium_profile(app: &Arc<App>, name: Option<String>) -> Result<Profile, Error> {
    let name = name.unwrap_or_else(|| app.settings().oneshot.profile.clone());

    let (found, engine) = app
        .store
        .run(move |c| {
            let found = profile::find(c, &name)?;
            let engine = resolve_build(c, &found.settings.build)?.engine;

            Ok((found, engine))
        })
        .await?;
    if engine == Engine::Camoufox {
        return Err(invalid(
            "profile",
            "/cdp serves Chromium-family profiles; Camoufox is driven through brr's operations (ADR 0003)",
        ));
    }

    Ok(found)
}

// A running Instance of a Persistent Profile, else a launch; the first Tab of a launch is
// returned so the connection can lease it.
async fn instance_for(
    app: &Arc<App>,
    mut profile: Profile,
    query: CdpQuery,
) -> Result<(InstanceId, Option<TabId>), Error> {
    let running = (profile.kind == ProfileKind::Persistent)
        .then(|| {
            app.instances.views().into_iter().find(|view| {
                view.instance.profile_id == Some(profile.id)
                    && matches!(
                        view.instance.state,
                        InstanceState::Running | InstanceState::Idle
                    )
            })
        })
        .flatten();
    if let Some(view) = running {
        if query.proxy.is_some() || query.display.is_some() {
            let given = if query.proxy.is_some() {
                "proxy"
            } else {
                "display"
            };

            return Err(invalid(
                given,
                "proxy and display apply only to a launch; this Profile is already running",
            ));
        }

        return Ok((view.instance.id, None));
    }

    if let Some(proxy) = &query.proxy {
        let proxy_id: ProxyId = proxy
            .parse()
            .map_err(|_| invalid("proxy", "use the id of a proxy"))?;
        profile.settings.proxy = Some(ProxyChoice::ProxyId(proxy_id));
    }
    // The launch outlives a dropped request; an Instance nobody owns is stopped.
    let (work_app, undo_app) = (Arc::clone(app), Arc::clone(app));
    let display = query.display;
    let (instance_id, first_tab) = detached(
        async move { launch_instance(&work_app, profile, display).await },
        move |(instance_id, _)| async move {
            if let Err(error) = undo_app
                .instances
                .stop(&undo_app, &instance_id, EndReason::Stop)
                .await
            {
                warn!(%instance_id, error = format!("{error:#}"), "abandoned cdp instance not stopped");
            }
        },
    )
    .await?;

    Ok((instance_id, Some(first_tab.id)))
}

async fn close(socket: &mut WebSocket, code: u16) {
    let frame = CloseFrame {
        code,
        reason: "".into(),
    };

    let _ = socket.send(Message::Close(Some(frame))).await;
}

async fn pump(
    mut socket: WebSocket,
    connection: Connection,
    instance_id: InstanceId,
    downstream: Downstream,
    mut outbound: mpsc::Receiver<Outbound>,
) {
    let app = Arc::clone(&connection.app);

    loop {
        tokio::select! {
            frame = socket.recv() => match frame {
                // Only what the Client sends is Activity: browser events would keep an idle
                // Instance alive for ever.
                Some(Ok(Message::Text(text))) => {
                    app.instances.activity(&instance_id, None);
                    downstream.send(text.as_str());
                }
                Some(Ok(Message::Binary(_))) => {
                    close(&mut socket, UNSUPPORTED).await;
                    break;
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break,
            },
            out = outbound.recv() => match out {
                Some(Outbound::Frame(text)) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Some(Outbound::BrowserClose) => {
                    // The stop is asked for before the socket closes, so the browser never exits
                    // on its own first: the Instance stops, it does not crash.
                    let stopping = Arc::clone(&app);
                    tokio::spawn(async move {
                        if let Err(error) =
                            stopping.instances.stop(&stopping, &instance_id, EndReason::Stop).await
                        {
                            warn!(%instance_id, error = format!("{error:#}"), "browser close not applied");
                        }
                    });
                    while let Ok(Outbound::Frame(text)) = outbound.try_recv() {
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    close(&mut socket, NORMAL_CLOSE).await;
                    break;
                }
                None => {
                    close(&mut socket, INTERNAL).await;
                    break;
                }
            },
        }
    }

    downstream.close().await;
    drop(connection);
}
