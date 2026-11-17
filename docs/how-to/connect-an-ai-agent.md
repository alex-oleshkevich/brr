# Connect an AI agent

brr speaks MCP two ways: `brr mcp` over stdio, and streamable HTTP at `http://127.0.0.1:9898/mcp`. Use stdio when the agent runs on the same machine as brr. Use HTTP when the client only supports URLs or runs elsewhere on a network you trust.

## Over stdio

Point your client at the `brr mcp` command. Most clients that take JSON (Claude Code's `.mcp.json`, Claude Desktop, Cursor and others) use this shape:

```json
{
  "mcpServers": {
    "brr": {
      "command": "brr",
      "args": ["mcp"]
    }
  }
}
```

For Codex, add this to `~/.codex/config.toml`:

```toml
[mcp_servers.brr]
command = "brr"
args = ["mcp"]
```

If `brr` is not on the client's `PATH`, use the full path, for example `/home/you/.local/bin/brr`.

`brr mcp` starts the daemon if it is not running and forwards every call to it. The agent keeps one session for as long as the client runs, so it has a current tab: after `profile_launch`, tab tools such as `navigate` and `get_markdown` work without a `tab_id`.

## Over HTTP

Give the client the URL:

```json
{
  "mcpServers": {
    "brr": {
      "type": "http",
      "url": "http://127.0.0.1:9898/mcp"
    }
  }
}
```

The HTTP endpoint keeps no session. Each call is a one-off unless the agent calls `client_create` first and passes the returned `id` as `client_id` on later calls. With a `client_id` the agent has a current tab and holds its leases between calls. A client that makes no call for 10 minutes ends (`client.mcp_http_idle_min`).

## Choose the tools

By default the agent sees the toolsets in the `mcp.default_toolsets` setting: `browser`, `instances`, `profiles`, `builds`, `network`, `credentials`, `captcha`, `storage` and `system`. Fewer tools leave the model more room. To expose only some:

```sh
brr mcp --toolsets browser,profiles
```

Over HTTP, add the same list as a query: `http://127.0.0.1:9898/mcp?toolsets=browser,profiles`.

To change the default for every client:

```sh
brr settings set mcp.default_toolsets browser,instances,profiles
```

## Teach the agent how to use brr

Install the brr skill for Claude Code and other agents that read `~/.agents/skills`:

```sh
brr skill install
```

This writes `SKILL.md` to `~/.claude/skills/brr` and `~/.agents/skills/brr`. Use `--dir <folder>` (repeatable) to install somewhere else. `brr skill` without a subcommand prints the skill.

At the start of a session, have the agent run:

```sh
brr prime
```

It prints a short guide to the tools and a summary of what is running right now.

## Agents without MCP

An agent that can run shell commands can use the CLI directly. Each `brr` command is its own client, so pass `--tab-id` on every tab command and `--json` when the agent parses output. See the [tutorial](../tutorial.md) for the pattern.
