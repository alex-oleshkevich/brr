# Live tests

Black-box tests of the running daemon over REST, MCP and the CLI. Bash, curl and jq only.

```
just live                 # every NN-*.sh in order
just live 07-leases       # one script (07, leases and 07-leases.sh also match)
```

`run.sh` builds once (`just build`), then runs each script against its own scratch daemon:
fresh `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME` and `XDG_RUNTIME_DIR` under `mktemp -d`,
`BRR_SERVER__BIND=127.0.0.1:<free port>` and `BRR_VAULT__KEY_SOURCE=file`. The daemon log is
`$XDG_STATE_HOME/brr/daemon.log`. Afterwards the daemon gets SIGTERM, any browser still carrying this
run's instance ids is a failure (and is killed), and the scratch dirs are removed. Nothing touches the real
user's config, data or daemon.

Each script prints one line: `PASS|FAIL|SKIP <name> <reason>`. The exit code is non-zero on any FAIL.
A script exits 77 to skip.

| Variable | Effect |
| --- | --- |
| `BRR_TEST_CHROME` | Path to a Chromium binary. It is linked as `google-chrome-stable` first on the scratch daemon's `PATH`, so `build.detect` registers it as the system Build. Unset: the host's Chrome/Chromium on `PATH` is used. |
| `BRR_LIVE_NETWORK=1` | Enables the real `cft stable` install in `04-builds` (up to 10 minutes) and the network parts of `20-builds` (headless_shell install, `build.available` for the other sources) and `22-extensions` (Web Store add, update, EngineUnsupported). Without it those parts are skipped and the script still prints PASS for what ran. |
| `BRR_TEST_CAMOUFOX` | Path to a Camoufox binary (`$HOME/.cache/brr-test/camoufox/camoufox-bin`, from `just camoufox-golden`); `40` to `42` and `44` use it. Unset or not executable: those parts SKIP with the reason. |
| `BRR_TEST_FPGEN_MODEL` | Directory of the unpacked fpgen model (`$HOME/.cache/brr-test/fpgen-model`); needed with `BRR_TEST_CAMOUFOX`. |
| `BRR_LIVE_RUSTFS_URL` | Where `need_rustfs` probes (default `http://127.0.0.1:9900`); point it at a dead port to see `34` skip. |
| `BRR_LIVE_TMPDIR` | Where the scratch dirs are made (default `/tmp`). Point it at a bigger disk for `44`, whose Camoufox install unpacks more than 2 GB. |
| `BRR_LIVE_KEEP=1` | Keeps each scratch dir (daemon log, script log) and prints its path. |
| `BRR_LIVE_NO_BUILD=1` | Skips `just build` and uses the existing `target/debug/brr`. |

Without any Chromium-family browser, `05` to `07`, `10`–`17` SKIP (`18` runs its browser-free part); `04` skips its detect part and
`09` its lease and stdio part.

| Script | Covers |
| --- | --- |
| `00-surfaces` | R1: every OpenAPI operation answered via REST, MCP and CLI; MCP tool list and `--help` for each |
| `01-errors` | §14: typed errors, exit codes 2/3/69, MCP `isError` text |
| `02-settings` | settings get/set, `config.toml`, `settings.changed`, env-pinned keys rejected |
| `03-profiles-templates` | built-ins, kinds, `name_taken`, `builtin`, managed args, templates copy by value |
| `04-builds` | detect, system Builds not installable/removable, optional network install with SSE progress |
| `05-instances-tabs` | launch, tabs, navigation error, `InvalidTransition`, ephemeral dir removal |
| `06-persistent-lock` | `profile_locked` while a persistent profile runs |
| `07-leases` | `LeaseHeld`, read-only access, exclusive Lease, client end, CLI SIGKILL releases |
| `08-events-sse` | SSE replay, `Last-Event-ID`, `kinds`, `events.list` paging and filters, `events.watch` |
| `09-mcp` | HTTP and stdio MCP, any `Host`, Client lifetime and Leases |
| `10-sigkill` | S1: `kill -9` the daemon, restart reconciles 5 browsers |
| `11-shutdown` | S2: SIGTERM stops every browser within `stop_timeout_s + 10` |
| `12-limits-static` | `limits.set`, reject/queue/cancel/evict admission, `Retry-After`, env-pinned limits, pinned Profiles never evicted |
| `13-admission-adaptive` | 20 concurrent launches under adaptive limits; the formula holds on every admitted decision |
| `14-idle-reaping` | Tab reaping, last Tab blanked (I2), Instance idle → reaping → stopped, activity during grace (several minutes) |
| `15-pin-last-tab` | pinned Tab kept; pinned Profile's visible Instance only goes idle (visible part needs `DISPLAY`/`WAYLAND_DISPLAY`) |
| `16-promote` | `instance.promote` keeps an ephemeral Instance's cookie in a new persistent Profile |
| `17-mcp-client-expiry` | quiet MCP HTTP client and closed stdio lose their Leases, then the Instances are reaped (several minutes) |
| `18-metrics-doctor` | `/metrics`, `/v1/system/doctor`, `brr doctor` without a daemon and its exit code |
| `19-daemon-install` | `brr daemon install/uninstall` against a fake `systemctl` |

Idle timeouts count whole minutes, so `14`, `15` and `17` take a few minutes each. A script can pin a
setting to the environment with a `# env: BRR_X__Y=1` comment line (`12` does this for a limit).
| `20-builds` | source/channel validation, system Builds not removable, doctor lists `xz`; network: headless_shell install, referenced Build refuses removal, other sources list versions |
| `21-proxies` | authenticated upstream proxy: health and exit ip, password never leaks, import counts, sticky pools, dead proxies (`ProxyUnhealthy`), referenced delete; the peer sees `127.0.0.2` (§18) |
| `22-extensions` | folder upload as tar, content script runs in a launched page, `.xpi` is a Camoufox extension, `../` tar entry refused; network: Web Store add/update, `EngineUnsupported` on headless_shell |
| `23-cookies-bookmarks` | offline cookie import sent by the browser, running export, bookmarks import/export, ephemeral profile refused |
| `24-profile-flags` | `webrtc` policy on the browser command line, `prefs` written to `Preferences` |
| `25-cli-files` | CLI file I/O: text export on stdout, `-o` file, 48 MB upload refused before any request |

Helpers live in `lib.sh`: `rest`, `brr`, `mcp_http`, `mcp_tool`, `need_chrome`, `assert_eq`,
`assert_status`, `assert_json`, `assert_log`, `wait_until`, `marker_pids`, `slow_server`; for `20`-`25` also `upstream_proxy`, `peer_server`, `trace_server`, `make_ext`, `profile_set`, `wait_title` (node servers live in `support/`). Every wait is a
bounded `wait_until`.

## M4 scripts (30–36)

| Script | Covers |
| --- | --- |
| `30-tab-tools` | every Tab tool on `fixtures/m4/form.html`, two Clients (`LeaseHeld`, read-only success), `StaleRef`, `ElementNotFound`, CLI `-o` and MCP |
| `31-oneshot` | `/v1/screenshot` (800x600 PNG), markdown, links, `return=url`, bad parameters, CLI, MCP; the ready pool (`BRR_ONESHOT__POOL_SIZE=1`) |
| `32-cdp` | `/cdp` through `cdp-client.ts` (raw WebSocket under Bun): targets, exclusive connect Lease, isolation, `Browser.close`, 423 on a held Lease |
| `33-artifacts-local` | pdf Artifact on local storage: public URL, list, fetch, delete |
| `34-storage-s3` | rustfs: wrong secret, secret hint, presigned 70 MB PDF, sweep deletes it |
| `35-retention` | the dry run lists exactly what the real sweep deletes |
| `36-quota-downloads` | disk floor, Profile quota, Download size limit and floor |

`34` needs rustfs at `http://127.0.0.1:9900` (access key `brradmin`, secret `brrsecret123`, bucket
`brr-artifacts`, path-style); it prints SKIP when nothing answers and never empties the bucket.
Chrome comes from `BRR_TEST_CHROME=$HOME/.cache/brr-test/chrome-linux64/chrome`. Fixtures are in
`fixtures/m4/` (`big-pdf.html` draws six noise canvases, about 70 MB of PDF; thirty exceed the daemon's
256 MiB capture cap). `lib.sh` adds `need_rustfs`, `http_server DIR [--download]` (port in
`$HTTP_PORT`), `cdp_ws URL SCRIPT_JSON` and `tab_call TAB TOOL JSON [CLIENT]`.

## M5 scripts (40–44)

| Script | Covers |
| --- | --- |
| `40-camoufox` | a Camoufox Instance: Tab tools, `EngineUnsupported` for `pdf`, `/cdp` refused, a persistent Profile keeps its cookie and its user agent |
| `41-virtual-display` | `display: virtual` sized to the Profile's screen, the Xvfb process carries the Instance marker, `DependencyMissing` without Xvfb, `camoufox-stealth` |
| `42-live-view` | the screencast WebSocket (`ws-client.ts`): frames, `LeaseHeld` for input, take over, click, key, dialog, hand back, `closed`, unknown `client_id` is 404 |
| `43-human-step-in` | `await_human`, UI input without a take over, `human-done`, the timeout, `client_gone`, `InvalidTransition` |
| `44-camoufox-install` | needs `BRR_LIVE_NETWORK=1` (about 2.2 GB download): the real install, assets, and either `build.installed` + `build.channel_moved` or `build.install_failed` consistent with `brr-compat.json` |

`40` to `44` seed the Camoufox Build and its fonts and model assets into the scratch database
(`seed_camoufox` in `lib.sh`, which restarts the daemon); nothing is written under `~/.cache/brr-test`.
`41` needs Xvfb on `PATH`. `ws-client.ts` takes one JSON message per stdin line and prints every received
message as a JSON line (frames reduced to their size); `lib.sh` wraps it as `ws_client`, `ws_send` and
`ws_close`, with `ws_probe` for refused upgrades.

## M7 scripts (60–63)

| Script | Covers |
| --- | --- |
| `60-live-cdp` | a Live browser attached by `cdp_url`: user Tabs (`opened_by_brr` false), `LeaseHeld`, user Tab close 400, `stop` 409, `detach`, re-attach, daemon SIGTERM, `lost` when the browser exits; the test Chrome stays alive throughout |
| `61-live-install-name` | attach by install name from a scratch `DevToolsActivePort`, REST and CLI, unknown name 400, missing file 503 with the `chrome://inspect` hint |
| `62-extension-relay` | the real companion extension in the test Chrome: `via: "extension"` attach shares only the fixture Tab, evaluate and `tab.open` round-trip, `stop` 409, Disconnect → `lost`; E3 companion Profile and `brr extension companion` zip. `BRR_LIVE_SLOW=1` adds the 30 s attach timeout |
| `63-backups` | backup passphrase never echoed, export → import on a second daemon (fresh data root) with the cookie, wrong passphrase 400, `keep 1`, rustfs backup/restore (skipped without rustfs), scheduled backup at the next minute (about 1–2 minutes) |

`lib.sh` adds `start_test_chrome [ARG...]` (scratch profile, `--remote-debugging-port=0`, sets `TEST_CHROME_PID/PORT/WS`), `extension_id DIR`,
`need_s3`, `start_second_daemon` with `rest2` and `brr2`. Fixtures are in `fixtures/m7/`. Manual only (the Chrome 144+ "Allow remote
debugging?" dialog cannot be scripted): (1) enable `chrome://inspect/#remote-debugging` in a real Chrome 144+, attach by `install_name`, click
Allow → attached; (2) click Deny → 409 `attach_declined`; (3) ignore it 30 s → 504 Timeout.

## M6 scripts (50–54)

| Script | Covers |
| --- | --- |
| `50-credentials` | vault CRUD over REST and CLI, no secret in any response or the daemon log, `totp:null`, `InvalidArgument` field `site`, Chrome CSV import counts, a Firefox file read as Chrome refused |
| `51-login` | `login` through MCP on a three-step site with TOTP, `credential_not_linked`, `LoginFailed` step 2, masking, `evaluate` refused on a page holding a filled secret, and the leak grep (`grep -rF` over every response, MCP result, event, the whole SSE stream, snapshot, html and `daemon.log`; it prints the bytes searched) |
| `52-login-checks` | a check fails at launch, `auto_relogin` makes it ok, `login_status` on the Profile, `profile_not_running` |
| `53-captcha` | auto-solve through `fake-2captcha.mjs`, token applied, solve log and balance, `daily_solves=1`: `awaiting_human` then `CaptchaFailed` after about 5 s, `CaptchaBudgetExceeded`, `/metrics` |
| `54-content` | ad blocking from a local filter list, `dismiss_cookies` on a one-shot; with `BRR_TEST_CAMOUFOX` and `BRR_TEST_FPGEN_MODEL` also a Camoufox Profile |

The fixtures are `fixtures/site.mjs` (login, captcha, ad and cookie pages) and `fixtures/fake-2captcha.mjs`
(`start_fixture NAME PORT` in `lib.sh`). Every URL is local. The scripts pin
`retention.free_disk_floor_gb` to 1 so a small `/tmp` does not refuse launches. Reading the masked field
back by OCR is a manual item.

## Error paths (90)

| Script | Covers |
| --- | --- |
| `90-error-paths` | §14 over REST, CLI and MCP for every op family: unknown ids on every path operation, unknown keys, malformed JSON, 413, `LeaseHeld` on every Tab tool, caps, state, quota, Build/proxy/dependency failures, login, captcha, backups, Live attach, Camoufox `EngineUnsupported`; no 5xx, no panic in `daemon.log`, no secret in any error text |

Unlike the other scripts it does not stop at the first failed assertion: it prints a table of every failed one (request, expected, actual) and a second table of spec ambiguities, which never fail the run.
