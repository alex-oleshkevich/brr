# Security model

## No authentication

brr has no login, no API tokens and no user accounts. The web UI, REST, MCP and the browser endpoints all answer anyone who can connect, and the daemon does not check `Host` or `Origin` headers. This is a deliberate choice, recorded in [ADR 0002](../adr/0002-no-auth-no-request-origin-checks.md).

brr is built to run on your own machine for your own agents and scripts, which reach it on localhost or over its unix socket. The boundary is the network: keep the daemon where only you can reach it.

What this means:

- Anything that can reach the port controls brr. It can launch browsers, run JavaScript in your logged-in profiles and make brr log in with your stored credentials.
- Because the daemon does not check `Origin`, a web page open in your everyday browser can send requests to `http://127.0.0.1:9898`.
- The local forwarder that adds proxy credentials listens on a loopback port, so any local user or process can send traffic through your configured proxies. A machine shared with users you do not trust is outside brr's model.

One route is the exception. The companion extension's relay (`/v1/extension-relay`) only accepts connections whose `Origin` is a browser extension. Without that check, a web page could pose as the extension and receive an agent's browser traffic.

## Keeping it safe

Keep `server.bind` at its default `127.0.0.1:9898`. Do not bind `0.0.0.0` on a machine that others can reach.

To use brr from another machine, choose one of these:

- An SSH tunnel: `ssh -L 9898:127.0.0.1:9898 your-server`, then use `http://127.0.0.1:9898` locally.
- A reverse proxy that does authentication (for example single sign-on or basic auth) in front of a daemon bound to 127.0.0.1. The proxy must pass WebSocket and server-sent event connections through for the web UI's live view and updates.

In Docker, publish the port to loopback only (`-p 127.0.0.1:9898:9898`); see [deploy/README.md](../../deploy/README.md).

## Secrets

brr keeps secrets in a vault inside its database: site credentials, proxy passwords, the Bitwarden API key and master password, captcha provider and storage keys, and the backup passphrase. Each is encrypted with a vault key. The key is either a file, `vault.key` in the data directory with mode 0600, or an entry in the system keyring. Which one is chosen when the vault is first created (`vault.key_source`, `file` by default) and cannot be changed later.

Back up the key file together with the data directory. Without it the stored secrets cannot be decrypted, and brr will not create a new key over an existing vault.

Secrets do not appear in logs, events or API responses. Listing credentials returns sites and usernames only, and proxy passwords show only whether they are set. When an agent logs in to a site, the daemon fills in the password and 2FA code itself; the agent sees the result, not the secret. Logins kept in Bitwarden are fetched when a login runs and are not copied into brr's database.

Profile exports and backups are encrypted with a passphrase.

## Sandbox modes

`sandbox.mode` decides how much a managed browser is separated from brr and the machine.

| | `none` (default) | `docker` |
|---|---|---|
| Runs as | brr's user, on brr's machine | its own container |
| Sees other files | yes, anything brr's user can read | only its own profile folder, downloads and extension copies; the browser program read-only |
| Network | direct | none of its own; only through brr's forwarder, which refuses localhost, LAN and other private addresses |
| Can see or signal brr | yes | no |
| Memory, CPU and process limits | no | yes (`sandbox.memory_mb`, `cpus`, `pids`, `shm_mb`) |

`none` is fine when you trust the sites you visit and you rely on the browser's own sandbox. Use `docker` when the browser visits untrusted sites, when the machine blocks Chromium's own sandbox, or when a page must not reach your LAN or brr's API.

`docker` has its own cost: brr needs access to the Docker socket, which is close to root on that machine. If brr is compromised, so is the host. Inside the container the browser runs without Chromium's own sandbox; the container is the boundary. Upstream proxy credentials stay in brr and never enter the container.

[Remote Hosts](../how-to/add-a-remote-host.md) always use containers. brr reaches each Host's Docker over SSH with the daemon user's keys, which is root-equivalent on that Host, and stores no SSH secrets. Browser traffic from a Host goes back through brr.

Browsers you attach to (`brr instance attach`) are never sandboxed; they are your own browsers.
