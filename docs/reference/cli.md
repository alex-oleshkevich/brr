# CLI reference

```
brr [OPTIONS] <COMMAND>
```

Every command except `brr daemon ...` starts the daemon if it is not running, then talks to it over its unix socket. `brr <command> --help` lists the flags of any command.

## Global options

| Option | Meaning |
|---|---|
| `--json` | Print the command's output as one line of JSON. |
| `-q`, `--quiet` | Log less. |
| `-v`, `--verbose` | Log more. Give it twice for trace logging. |
| `-h`, `--help` | Print help. |

## Command groups

| Group | Purpose |
|---|---|
| `daemon` | Run and manage the daemon: `run` (foreground), `start`, `stop`, `status`, `install` and `uninstall` (systemd user service), `logs`. |
| `mcp` | Serve MCP over stdio, forwarding to the daemon. `--toolsets` limits the tools. |
| `doctor` | Check this machine and the daemon's setup. Same as `system doctor`. |
| `prime` | Print the agent guide and what is running right now. |
| `skill` | Print the brr agent skill, or `install` it for coding agents. |
| `version` | Print the version. |
| `admission` | List launches waiting for room (`queue`). |
| `artifact` | List, get, fetch and delete stored files: screenshots, PDFs, downloads, HARs. |
| `backup` | Back up stopped persistent profiles to the storage backend, list, restore and delete backups. |
| `batch` | Run several browser tools in order in one call (`run --steps`). |
| `bitwarden` | Connect a Bitwarden or Vaultwarden account, list its items, link them to profiles, sync. |
| `build` | Install, detect, list and remove browser builds; list what a source offers. |
| `captcha` | Captcha provider balance, settings, solve log and daily stats. |
| `client` | Create, list and end clients that hold leases. |
| `credential` | Store, import, list, change and delete logins in the vault. |
| `events` | List stored events, or `watch` / `--follow` for new ones. |
| `extension` | Add, list, update and remove browser extensions; download the companion extension. |
| `host` | Add and manage remote machines that run browsers in Docker over SSH. |
| `instance` | List, get and stop running browsers; attach to and detach from a browser you started; promote an ephemeral run to a profile. |
| `lease` | Acquire, list and release leases on tabs and instances. |
| `limits` | Show and change instance and tab limits and admission defaults. |
| `login-check` | Pages that prove a profile is logged in: create, list, run, change, delete. |
| `pool` | Proxy pools, sticky per profile or rotated per launch. |
| `profile` | Create, list, change, launch, export, import and delete profiles; their cookies, bookmarks and extensions. |
| `proxy` | Add, import, check, change and delete proxies. |
| `retention` | Show and change how long stored data is kept; sweep now. |
| `settings` | Read and change settings (`get`, `set`). |
| `shot` | One-off captures of a URL with no profile: `markdown`, `content`, `links`, `screenshot`, `pdf`, `mhtml`. |
| `storage` | The storage backend for artifacts and backups (`backend`), and disk use (`usage`). |
| `system` | Daemon version, memory and load (`info`); machine checks (`doctor`). |
| `tab` | Drive a tab: navigate, snapshot, click, fill, type, read text or Markdown, screenshot, PDF, cookies, storage, network and console logs, log in, solve captchas, and more. |
| `template` | Create, list, change and delete profile templates; save one from a profile. |
| `usage` | Browsers running over time (`history`) and proxy traffic (`traffic`). |

## Flag conventions

- Each CLI call is its own client. Nothing carries over between commands, so tab commands need `--tab-id` and instance commands need the instance id.
- Commands that write a file take `-o, --output <PATH>`, for example `brr tab screenshot -o page.png`. `brr shot screenshot`, `pdf` and `mhtml` require it.
- Boolean flags are switches that also accept a value: `--full-page` or `--full-page=false`.
- List flags are repeated: `--profile-ids a --profile-ids b`.
- Some flags take JSON, for example `--settings` on `profile update` and `--selectors` on `credential create`.
- `update` commands change only the flags you pass. `--clear-<field>` resets a field.
- A positional `<profile>` or `<host>` accepts a name or an id.

## The CLI, MCP and REST are the same operations

Each CLI command is one operation of the daemon. The same operation is an MCP tool and a REST route, with the same inputs and outputs:

| CLI | MCP tool | REST |
|---|---|---|
| `brr profile create` | `profile_create` | `POST /v1/profiles` |
| `brr profile launch <profile>` | `profile_launch` | `POST /v1/profiles/{profile}/launch` |
| `brr tab navigate <url>` | `navigate` | `POST /v1/tabs/{tab_id}/navigate` |
| `brr tab get-markdown` | `get_markdown` | `POST /v1/tabs/{tab_id}/get_markdown` |
| `brr instance stop <id>` | `instance_stop` | `POST /v1/instances/{instance_id}/stop` |
| `brr shot markdown <url>` | `oneshot_markdown` | `POST /v1/oneshot/markdown` |
| `brr settings set` | `settings_set` | `PATCH /v1/settings` |

Tab tools drop the `tab` prefix in MCP. `/openapi.json` on the daemon lists every route. See [HTTP API](http-api.md).

## Exit codes

| Code | Error |
|---|---|
| 0 | Success |
| 1 | `Internal` |
| 2 | `InvalidArgument`, `RequestTooLarge`, and invalid command-line arguments |
| 3 | `NotFound` |
| 4 | `Conflict` |
| 5 | `InvalidTransition` |
| 6 | `LeaseHeld` |
| 7 | `LeaseLost` |
| 8 | `CapExceeded` |
| 9 | `QuotaExceeded` |
| 10 | `DownloadTooLarge` |
| 11 | `BuildMissing` |
| 12 | `EngineUnsupported` |
| 13 | `DependencyMissing` |
| 14 | `ProxyUnhealthy` |
| 15 | `NavigationFailed` |
| 16 | `ElementNotFound`, `StaleRef` |
| 17 | `EvaluationFailed` |
| 18 | `LoginFailed` |
| 19 | `CaptchaBudgetExceeded` |
| 20 | `CaptchaFailed` |
| 21 | `StorageUnavailable` |
| 22 | `Timeout` |
| 23 | `BitwardenLocked` |
| 69 | The daemon is not reachable |

Errors print to stderr as `error: <message>`, often followed by a `help:` line with the fix.
