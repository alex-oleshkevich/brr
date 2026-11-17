# Configuration reference

## Where settings come from

Each setting has a key of the form `section.key`. brr reads it from three places; a later one wins:

1. The built-in default.
2. The config file, `$XDG_CONFIG_HOME/brr/config.toml` (usually `~/.config/brr/config.toml`).
3. An environment variable `BRR_<SECTION>__<KEY>` in the daemon's environment, for example `BRR_SERVER__BIND=0.0.0.0:9898`, `BRR_SANDBOX__MODE=docker` or `BRR_VAULT__KEY_SOURCE=file`. The value is read as a TOML value, or as a string if it is not one. An unknown name stops the daemon from starting.

## Reading and changing settings

```sh
brr settings get                        # every setting and where its value came from
brr settings set idle.tab_min 20        # writes config.toml
brr settings set sandbox.memory_mb 4096 sandbox.cpus 4
brr settings set idle.tab_min null      # back to the default
```

`settings set` takes key and value pairs. A value is read as JSON, or as a string if it is not JSON. A comma-separated value sets a list: `brr settings set mcp.default_toolsets browser,profiles`. Changes apply at once, except `server.bind` and `server.socket`, which need a daemon restart. The same operations are `GET` and `PATCH /v1/settings`, and the MCP tools `settings_get` and `settings_set`.

The config file is TOML with one table per section:

```toml
[idle]
tab_min = 20

[sandbox]
mode = "docker"
image = "brr:dev"
```

## Settings

| Key | Default | Meaning |
|---|---|---|
| `server.bind` | `127.0.0.1:9898` | Address and port of the web UI, REST and MCP over HTTP. Restart to apply. |
| `server.socket` | `null` | Unix socket path; `null` means `$XDG_RUNTIME_DIR/brr/brr.sock`. Restart to apply. |
| `limits.mode` | `static` | `static` uses only the counts below; `adaptive` also checks free memory and CPU load. |
| `limits.max_instances` | 8 | Browsers running at once. |
| `limits.max_instances_per_engine` | `{chromium = 8, camoufox = 4}` | Browsers running at once per engine. |
| `limits.max_instances_per_tag` | `{}` | Browsers running at once per profile tag. |
| `limits.max_tabs_per_instance` | 20 | Tabs in one browser. |
| `limits.max_tabs_total` | 100 | Tabs across all browsers. |
| `adaptive.min_free_ram_mb` | 1024 | In adaptive mode, memory that must stay free after a launch. |
| `adaptive.max_load_per_core` | 0.85 | In adaptive mode, highest 1-minute load average per CPU that still admits a launch. |
| `adaptive.reserve_mb` | `{chromium_headless = 400, chromium_display = 600, camoufox = 600}` | First memory estimate per browser kind, before brr has measured one. |
| `adaptive.pressure_check_s` | 5 | How often adaptive mode checks memory and load, in seconds. |
| `admission.default` | `queue` | What a launch does when there is no room: `queue`, `reject` or `evict` (close an idle browser). |
| `admission.priority` | 5 | Default launch priority, 0 to 9; higher goes first. |
| `admission.queue_timeout_s` | 60 | How long a launch waits in the queue. |
| `instance.start_timeout_s` | 30 | How long a browser may take to start. |
| `instance.stop_timeout_s` | 10 | How long a browser may take to stop before it is killed. |
| `instance.auto_restart` | `false` | Restart a browser that crashed. |
| `idle.tab_min` | 15 | Minutes before an idle tab is closed. |
| `idle.instance_min` | 30 | Minutes before an idle browser is stopped. |
| `idle.grace_s` | 60 | Warning period before closing an idle tab or browser; any activity cancels it. |
| `lease.exclusive_wait_s` | 60 | How long a request for an exclusive lease on a browser waits for other clients to release theirs. |
| `client.rest_idle_min` | 10 | Minutes before an idle REST client ends. |
| `client.mcp_http_idle_min` | 10 | Minutes before an idle MCP-over-HTTP client ends. |
| `mcp.default_toolsets` | `browser, instances, profiles, builds, network, credentials, captcha, storage, system` | Toolsets an MCP client sees unless it asks for others. |
| `oneshot.profile` | `oneshot` | Profile that `brr shot` and the one-shot routes use. |
| `oneshot.pool_size` | 0 | Browsers kept started for one-shot requests. |
| `oneshot.linger_s` | 60 | Seconds a one-shot browser stays open after its last request. |
| `results.inline_max_mb` | 5 | Larger results are stored as artifacts instead of returned inline. |
| `artifacts.url_ttl_s` | 3600 | Lifetime of artifact download URLs, in seconds. |
| `tab.console_buffer` | 1000 | Console messages kept per tab. |
| `tab.network_buffer` | 1000 | Network requests kept per tab. |
| `login_check.interval_min` | 60 | Minutes between login checks. |
| `proxy.health_interval_min` | 60 | Minutes between proxy health checks. |
| `proxy.check_url` | `https://www.cloudflare.com/cdn-cgi/trace` | Page a proxy check fetches to find the exit IP. |
| `captcha.default_policy` | `auto` | Captcha policy for profiles that set none: `off`, `on_demand`, `auto` or `human`. |
| `captcha.daily_usd` | 5.0 | Daily spending limit for the captcha provider, in US dollars. |
| `captcha.daily_solves` | 200 | Daily limit of paid solves. |
| `captcha.human_wait_s` | 300 | How long to wait for a person to solve a captcha. |
| `captcha.provider_url` | `https://api.2captcha.com` | Captcha provider API. |
| `content.adblock_lists` | EasyList and EasyPrivacy URLs | Filter lists used when a profile blocks ads. |
| `builds.check_at` | `04:00` | Local time of the daily check for new browser versions. |
| `builds.github_api` | `https://api.github.com` | GitHub API used to find builds. |
| `builds.edge_repo` | `https://packages.microsoft.com/repos/edge` | Repository Edge builds come from. |
| `extensions.cws_update_url` | `https://clients2.google.com/service/update2/crx` | Chrome Web Store download endpoint. |
| `extensions.amo_api` | `https://addons.mozilla.org/api/v5` | Firefox add-ons API. |
| `backup.keep` | 7 | Backups kept per profile. |
| `retention.artifacts_max_age_days` | 7 | Artifacts older than this are deleted. |
| `retention.artifacts_max_total_gb` | 10 | Above this total, the oldest artifacts are deleted. |
| `retention.download_max_mb` | 2048 | Larger downloads are cancelled. |
| `retention.free_disk_floor_gb` | 5 | No launches or downloads when free disk is below this. |
| `retention.unused_days` | 30 | Builds and extensions unused this long are deleted. |
| `retention.events_days` | 30 | Events and the captcha solve log are kept this long. |
| `sandbox.mode` | `none` | `none` or `docker`. See [Isolate browsers in Docker](../how-to/isolate-browsers-in-docker.md). |
| `sandbox.image` | `null` | brr image for browser containers; `null` means the image brr itself runs in. |
| `sandbox.memory_mb` | 2048 | Memory limit per container. |
| `sandbox.cpus` | 2.0 | CPU limit per container. |
| `sandbox.pids` | 1024 | Process limit per container. |
| `sandbox.shm_mb` | 1024 | `/dev/shm` size per container. |
| `storage.backend` | `local` | Where artifacts and backups go: `local`, S3-compatible or Cloudflare R2. Change it with `brr storage backend set`, which tests the connection first. |
| `storage.endpoint`, `storage.region`, `storage.bucket`, `storage.access_key_id`, `storage.path_style`, `storage.r2_account_id`, `storage.public_endpoint` | unset, `path_style = false` | S3 and R2 connection details. The secret key is kept in the vault, not here. |
| `bitwarden.server_url` | `https://vault.bitwarden.com` | Default Bitwarden server. Must be https, except for localhost. |
| `bitwarden.sync_interval_min` | 15 | Minutes between Bitwarden syncs. |
| `vault.key_source` | `file` | Where the vault key lives: `file` or `keyring`. Chosen when the vault is created and fixed after that. |
| `ui.theme` | `system` | Web UI theme: `system`, `light` or `dark`. |

## Paths

brr follows the XDG base directories. With none set:

| What | Path |
|---|---|
| Config file | `~/.config/brr/config.toml` |
| Data: profiles, builds, database (`brr.db`), vault key (`vault.key`), artifacts | `~/.local/share/brr/` |
| Daemon log | `~/.local/state/brr/daemon.log` |
| Browser logs | `~/.local/state/brr/instances/<instance id>.log` |
| Socket | `$XDG_RUNTIME_DIR/brr/brr.sock`, or `/tmp/brr-<uid>/brr.sock` when `XDG_RUNTIME_DIR` is unset |

`brr daemon status` prints the config path and data directory the running daemon uses. The CLI finds the daemon through the socket, so a CLI and a daemon with different XDG variables do not see each other.
