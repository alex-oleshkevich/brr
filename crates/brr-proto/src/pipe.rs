use command_fds::{CommandFdExt, FdMapping};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::process::Stdio;

use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::frames::{Link, Side, Stream};
use crate::{Error, Incoming, Mux};

const COMMANDS_FD: i32 = 3;
const MESSAGES_FD: i32 = 4;

pub struct Spawned {
    pub child: Child,
    pub mux: Mux,
    pub events: mpsc::Receiver<Incoming>,
    // The sandbox runner's framed connection, for the streams brr opens; None without a runner.
    pub link: Option<Link>,
    // Streams the runner opens: proxy tunnels, and a remote run's Profile copy-back.
    pub opened: Option<mpsc::Receiver<Stream>>,
}

pub fn spawn(mut command: Command) -> Result<Spawned, Error> {
    // Juggler shuts these fds down with shutdown(2), which only works on sockets.
    let (commands_read, commands_write) = StdUnixStream::pair()?;
    let (messages_read, messages_write) = StdUnixStream::pair()?;

    command
        .fd_mappings(vec![
            FdMapping {
                parent_fd: commands_read.into(),
                child_fd: COMMANDS_FD,
            },
            FdMapping {
                parent_fd: messages_write.into(),
                child_fd: MESSAGES_FD,
            },
        ])
        .map_err(std::io::Error::other)?;
    let child = command.spawn()?;

    // The command owns the child's pipe ends. While the parent holds the write end of fd 4 the
    // reader never sees EOF when the browser exits.
    drop(command);

    commands_write.set_nonblocking(true)?;
    messages_read.set_nonblocking(true)?;
    let (reader, _) = UnixStream::from_std(messages_read)?.into_split();
    let (_, writer) = UnixStream::from_std(commands_write)?.into_split();
    let (mux, events) = Mux::start(reader, writer);

    Ok(Spawned {
        child,
        mux,
        events,
        link: None,
        opened: None,
    })
}

// For a wrapper (the sandbox's `docker run`) whose stdin and stdout carry frames: stream 0 relays
// the browser's fds 3 and 4; other streams carry proxy tunnels and file transfers.
pub fn spawn_stdio(mut command: Command) -> Result<Spawned, Error> {
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = command.spawn()?;

    let writer = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("the child has no stdin"))?;
    let reader = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("the child has no stdout"))?;
    let (link, pipe, opened) = Link::start(reader, writer, Side::Daemon);
    let (reader, writer) = tokio::io::split(pipe);
    let (mux, events) = Mux::start(reader, writer);

    Ok(Spawned {
        child,
        mux,
        events,
        link: Some(link),
        opened: Some(opened),
    })
}
