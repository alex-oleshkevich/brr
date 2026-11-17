# Isolate browsers in Docker

With `sandbox.mode` set to `docker`, brr runs each managed browser in its own container. The container sees only its own profile folder and downloads, has no network except through brr, and has memory, CPU and process limits. See [Security model](../explanation/security.md#sandbox-modes) for what this protects against.

This guide is about the browsers brr launches. To run brr itself in a container, see [deploy/README.md](../../deploy/README.md).

## Before you start

- Docker is installed and the user running brr can use it (`docker version` works without `sudo`). Access to the Docker socket is close to root on that machine.
- You have a brr image of the same version as your `brr` binary. When brr runs inside the brr image it uses that image. When brr runs directly on the host, you must name one in `sandbox.image`. Build one from the repository with `just docker-build`, which tags it `brr:dev`.

## Turn it on

```sh
brr settings set sandbox.mode docker sandbox.image brr:dev
```

brr checks Docker before it saves `docker`; if Docker is not usable, the command fails and says why. The change applies to browsers launched after it. Browsers already running stay as they are.

Check that it took:

```sh
brr doctor
```

```
    sandbox          ok  browsers run isolated in docker image brr:dev
```

Launch a profile as usual. The instance record shows `sandbox  docker`, and `docker ps` lists a container named `brr-<instance id>`. brr removes the container when the browser stops.

## Set the limits

Each container gets these limits by default:

| Setting | Default |
|---|---|
| `sandbox.memory_mb` | 2048 |
| `sandbox.cpus` | 2.0 |
| `sandbox.pids` | 1024 |
| `sandbox.shm_mb` | 1024 |

Change them like any setting:

```sh
brr settings set sandbox.memory_mb 4096 sandbox.cpus 4
```

## Things that work differently

- `visible` display is not available. Use `headless`, or `virtual` for a browser that needs a screen; brr runs Xvfb inside the container.
- Pages cannot reach localhost, your LAN or other private addresses. Only public addresses work.
- Browsers you attach with `brr instance attach` are never sandboxed.

## Turn it off

```sh
brr settings set sandbox.mode null sandbox.image null
```

`null` resets a setting to its default, which for `sandbox.mode` is `none`.
