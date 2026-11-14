use std::{
    fs,
    future::IntoFuture,
    io,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    sync::Arc,
    time::Duration,
};

use brr_core::Error;
use futures::future::join;
use tokio::{
    net::{TcpListener, UnixListener},
    signal::unix::{SignalKind, signal},
    sync::watch,
    time,
};

use crate::{app::App, http, internal};

const SOCKET_MODE: u32 = 0o600;
// Graceful shutdown waits for every open connection, and SSE and presence connections only end
// when the client leaves; after this long the servers are dropped, which closes them.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

pub async fn run(app: Arc<App>) -> Result<(), Error> {
    let mut terminate = signal(SignalKind::terminate()).map_err(internal)?;
    let mut interrupt = signal(SignalKind::interrupt()).map_err(internal)?;

    serve(app, async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
    })
    .await
}

pub async fn serve(
    app: Arc<App>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Error> {
    let settings = app.settings();
    let bind = settings.server.bind;
    let socket = app.paths.socket(&settings);

    let tcp = TcpListener::bind(bind).await.map_err(|e| {
        internal(format!(
            "cannot listen on {bind}: {e}; set server.bind to a free address"
        ))
    })?;
    // The daemon lock is held, so a socket left here is stale; anything else is not ours to delete.
    match fs::symlink_metadata(&socket) {
        Ok(found) if found.file_type().is_socket() => {
            fs::remove_file(&socket).map_err(internal)?;
        }
        Ok(_) => {
            return Err(internal(format!(
                "{} exists and is not a socket; remove it or set server.socket elsewhere",
                socket.display()
            )));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(internal(e)),
    }
    let unix = UnixListener::bind(&socket).map_err(|e| {
        internal(format!(
            "cannot listen on {}: {e}; set server.socket to another path",
            socket.display()
        ))
    })?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(SOCKET_MODE)).map_err(internal)?;

    tracing::info!(tcp = %tcp.local_addr().map_err(internal)?, socket = %socket.display(), "listening");

    let router = http::router(Arc::clone(&app));
    let (stop_tx, stop_rx) = watch::channel(false);
    let stopped = |mut stop: watch::Receiver<bool>| async move {
        let _ = stop.wait_for(|stop| *stop).await;
    };

    let tcp_server = axum::serve(
        tcp,
        router
            .clone()
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(stopped(stop_rx.clone()));
    let unix_server = axum::serve(unix, router).with_graceful_shutdown(stopped(stop_rx));
    let servers = join(tcp_server.into_future(), unix_server.into_future());
    tokio::pin!(servers);

    let draining = async {
        shutdown.await;
        tracing::info!("shutting down");
        app.shutdown().await;
        let _ = stop_tx.send(true);
    };

    let result = tokio::select! {
        (tcp, unix) = &mut servers => tcp.and(unix).map_err(internal),
        () = draining => {
            let _ = time::timeout(DRAIN_TIMEOUT, servers).await;

            Ok(())
        }
    };

    if let Err(error) = fs::remove_file(&socket) {
        tracing::warn!(error = format!("{error:#}"), "socket file not removed");
    }

    result
}
