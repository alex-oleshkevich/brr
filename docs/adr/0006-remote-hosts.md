# Remote Hosts over Docker and SSH

brr runs browsers on other machines by driving their Docker over SSH (`docker -H ssh://…`); nothing
of brr is installed there. A remote browser runs in a container exactly like a local docker-sandboxed
one, and the container's stdio carries a framed stream protocol: the browser pipe, the browser's
proxy tunnels back to brr's forwarder, and file transfers (Profile copy in/out, downloads). One
daemon keeps the only database, the only API and the only connection to each browser
([ADR 0004](0004-one-pipe-connection-per-instance.md)).

We chose this over a brr agent on each machine (another process to deploy, upgrade and secure) and
over shared storage (NFS/rsync of the data dir): the per-run copy is bounded and needs only Docker
and SSH on the Host. Routing all browser traffic back through brr keeps proxy credentials, the
LAN/loopback block and traffic metering in one place, at the cost of bandwidth through brr's machine.

## Consequences

- SSH access to a Host's Docker is root-equivalent on that Host; brr uses the daemon user's SSH keys
  and stores no SSH secrets.
- A lost connection ends the run; the Profile keeps its pre-run copy.
- Do not give remote containers their own network route or send proxy credentials to a Host without
  revisiting this decision.
