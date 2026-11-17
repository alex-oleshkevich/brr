# HTTP API reference

The daemon serves everything on one address, `http://127.0.0.1:9898` by default (setting `server.bind`):

| Path | What |
|---|---|
| `/` | Web UI |
| `/v1/...` | REST API |
| `/openapi.json` | OpenAPI description of every REST route |
| `/mcp` | MCP over streamable HTTP |
| `/v1/events/stream` | Server-sent events for everything that happens |

The same API is also on the unix socket `$XDG_RUNTIME_DIR/brr/brr.sock` (setting `server.socket`), which is what the CLI uses:

```sh
curl --unix-socket "$XDG_RUNTIME_DIR/brr/brr.sock" http://localhost/v1/system/info
```

No route requires authentication, and the daemon does not check `Host` or `Origin` headers. See [Security model](../explanation/security.md).

## REST

Every CLI command and MCP tool has a REST route with the same input and output. Bodies are JSON. The full list, with schemas, is at `/openapi.json`:

```sh
curl -s http://127.0.0.1:9898/openapi.json | jq -r '.paths | keys[]'
```

Examples:

```sh
curl -s http://127.0.0.1:9898/v1/profiles
curl -s -X POST http://127.0.0.1:9898/v1/profiles/my-first/launch \
  -H 'content-type: application/json' -d '{"url": "https://example.com"}'
curl -s -X POST http://127.0.0.1:9898/v1/tabs/<tab id>/get_markdown \
  -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:9898/v1/instances/<instance id>/stop \
  -H 'content-type: application/json' -d '{}'
```

List routes take `?limit=` and `?cursor=`; the response has `next_cursor` when there is more.

### Clients and the `Brr-Client` header

A request without a `Brr-Client` header runs as a one-off client: any lease it takes ends with the request. To hold leases across requests, create a client and send its id on each request:

```sh
curl -s -X POST http://127.0.0.1:9898/v1/clients \
  -H 'content-type: application/json' -d '{"label": "my-script"}'
# -> {"id": "01M7...", "kind": "rest", "label": "my-script", ...}

curl -s -X POST http://127.0.0.1:9898/v1/profiles/my-first/launch \
  -H 'Brr-Client: 01M7...' -H 'content-type: application/json' -d '{}'
```

The client ends after `client.rest_idle_min` (10) minutes without a request, or when you delete it with `DELETE /v1/clients/{client_id}`. Its leases are released then.

### Errors

Errors have an HTTP status and a JSON body:

```json
{"code": "NotFound", "message": "profile nope not found", "details": {"what": "profile nope"}}
```

| Status | Codes |
|---|---|
| 400 | `InvalidArgument` |
| 402 | `CaptchaBudgetExceeded` |
| 404 | `NotFound` |
| 409 | `Conflict`, `InvalidTransition`, `BitwardenLocked` |
| 410 | `LeaseLost` |
| 413 | `DownloadTooLarge`, `RequestTooLarge` |
| 422 | `EngineUnsupported`, `ElementNotFound`, `StaleRef`, `EvaluationFailed`, `LoginFailed`, `CaptchaFailed` |
| 423 | `LeaseHeld` |
| 424 | `BuildMissing` |
| 429 | `CapExceeded` |
| 500 | `Internal` |
| 502 | `ProxyUnhealthy`, `NavigationFailed` |
| 503 | `DependencyMissing`, `StorageUnavailable` |
| 504 | `Timeout` |
| 507 | `QuotaExceeded` |

The CLI maps the same codes to [exit codes](cli.md#exit-codes).

## MCP over HTTP

`POST /mcp` speaks MCP streamable HTTP without sessions. The tools are the same as `brr mcp` serves over stdio.

- Add `?toolsets=browser,profiles` to the URL to expose only some toolsets. The default is the `mcp.default_toolsets` setting.
- A tool call without `client_id` runs as a one-off client. Call `client_create` first and pass the returned `id` as the `client_id` argument of later calls to keep a current tab and hold leases. The client ends after `client.mcp_http_idle_min` (10) minutes without a call, or with `client_end`.

See [Connect an AI agent](../how-to/connect-an-ai-agent.md) for client configuration.
