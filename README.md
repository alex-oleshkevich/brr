# brr

brr is a daemon that installs real browsers, runs them from saved profiles and keeps track of their tabs. You drive it from a terminal, from an AI agent over MCP, from your own code over REST, or from the web UI it serves. It is for people who automate the web and for coding agents that need a real browser, on a laptop or on a Linux server.

- Installs Chromium-family browsers (Chrome for Testing, Chromium, Brave, Edge and others) and Camoufox, a Firefox-based anti-detect browser.
- Profiles keep cookies, logins and settings between runs, or start fresh each time.
- Every action is a CLI command, an MCP tool and a REST route.
- Closes browsers that sit idle and limits how many run at once.
- Proxies and proxy pools, with traffic counted per profile and per proxy.
- An encrypted vault for site logins, plus Bitwarden; agents can log in without seeing the password.
- Optional Docker sandbox for each browser, and remote machines that run browsers over SSH.
- Web UI with a live view, so a person can step in to solve a captcha or log in.

## Install

Linux on x86_64 or aarch64:

```sh
curl -fsSL https://raw.githubusercontent.com/alex-oleshkevich/brr/main/install.sh | sh
```

This puts `brr` in `~/.local/bin`. Set `BRR_INSTALL_DIR` to install somewhere else, or `BRR_VERSION=0.1.0` to pin a version.

From source (needs Rust 1.96 or newer, bun and just):

```sh
just install
```

## Quick start

```sh
brr build install --source cft          # download Chrome for Testing
brr build list                          # repeat until the build says "installed"
brr profile create my-first --template chromium-headless
brr profile launch my-first --url https://example.com
TAB=$(brr tab list --json | jq -r '.tabs[0].id')
brr tab get-markdown --tab-id "$TAB"
```

The first brr command starts the daemon. Open http://127.0.0.1:9898 to see the running browser in the web UI.

## Documentation

Tutorial:

- [Your first browser](docs/tutorial.md)

How-to guides:

- [Run brr as a service](docs/how-to/run-as-a-service.md)
- [Connect an AI agent](docs/how-to/connect-an-ai-agent.md)
- [Isolate browsers in Docker](docs/how-to/isolate-browsers-in-docker.md)
- [Run browsers on another machine](docs/how-to/add-a-remote-host.md)
- [Use a proxy](docs/how-to/use-a-proxy.md)
- [Save logins](docs/how-to/save-logins.md)
- [Run brr itself in Docker](deploy/README.md)
- [Drive your everyday Chrome with the companion extension](extension/README.md)

Reference:

- [CLI](docs/reference/cli.md)
- [Configuration](docs/reference/configuration.md)
- [HTTP API and MCP endpoint](docs/reference/http-api.md)

Explanation:

- [Concepts](docs/explanation/concepts.md)
- [Security model](docs/explanation/security.md)
- [Glossary](GLOSSARY.md)

## Security

brr has no authentication. Anyone who can reach its port can launch browsers, run JavaScript in your logged-in profiles and use your stored logins. That includes web pages open in your own browser, because brr does not check where requests come from. It listens on 127.0.0.1 by default; keep it there, or put your own authenticating proxy in front of it. See [Security model](docs/explanation/security.md).

## License

BSD-3-Clause. See [LICENSE](LICENSE).
