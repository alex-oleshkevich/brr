use anyhow::Context;
use brr_core::{Error, Toolset};
use brr_daemon::{
    Boot,
    mcp::{Backend, Ensure, Forwarder, McpServer},
};
use clap::{Arg, ArgMatches, Command};
use rmcp::{ServiceExt, transport::stdio};
use tokio::signal::unix::{SignalKind, signal};

use super::daemon;

pub const NAME: &str = "mcp";

const TOOLSETS: &str = "toolsets";
const CLIENT_LABEL: &str = "stdio";

pub fn command() -> Command {
    Command::new(NAME)
        .about("Serve MCP over stdio, forwarding to the daemon")
        .arg(
            Arg::new(TOOLSETS).long(TOOLSETS).help(
                "Comma-separated toolsets to expose; default: the mcp.default_toolsets setting",
            ),
        )
}

pub async fn run(boot: &Boot, matches: &ArgMatches) -> anyhow::Result<()> {
    let toolsets = match matches.get_one::<String>(TOOLSETS) {
        Some(list) => Toolset::parse_list(list)?,
        None => boot.settings.mcp.default_toolsets.clone(),
    };

    let mut terminate = signal(SignalKind::terminate())?;

    let remote = daemon::ensure_daemon(&boot.paths, &boot.settings).await?;
    // A daemon restart ends this Client; the next call starts the daemon if needed and registers a new one.
    let (paths, settings) = (boot.paths.clone(), boot.settings.clone());
    let ensure: Ensure = Box::new(move || {
        let (paths, settings) = (paths.clone(), settings.clone());

        Box::pin(async move {
            daemon::ensure_daemon(&paths, &settings)
                .await
                .map(drop)
                .map_err(|error| {
                    error
                        .downcast::<Error>()
                        .unwrap_or_else(|error| Error::Internal {
                            message: format!("{error:#}"),
                        })
                })
        })
    });
    let forwarder = Forwarder::new(remote, CLIENT_LABEL, ensure);
    // While the forwarder's Hold lives the daemon keeps this Client; when the process ends the
    // socket closes and the daemon releases its Leases.
    forwarder.client_id().await?;

    let server = McpServer::new(Backend::Forward(forwarder), Some(toolsets));
    let running = server
        .serve(stdio())
        .await
        .context("cannot start the MCP server")?;

    // On SIGTERM the service and the hold drop, so the daemon ends the Client at once.
    tokio::select! {
        ended = running.waiting() => { ended?; }
        _ = terminate.recv() => {}
    }

    Ok(())
}
