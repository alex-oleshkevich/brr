# Running brr in Docker

The image runs the daemon as the non-root user `brr` under `tini`, with Xvfb, Debian's Chromium and the
libraries Camoufox needs. The daemon binds `0.0.0.0:9898` inside the container (required for `-p`).

```sh
just docker-seccomp   # only to regenerate deploy/seccomp-chrome.json; the file is committed
just docker-build     # builds brr:dev

docker run -d --name brr --shm-size=2g -p 127.0.0.1:9898:9898 -v brr-data:/data \
  --security-opt seccomp=deploy/seccomp-chrome.json brr:dev
```

| Flag | Why |
|---|---|
| `--shm-size=2g` | Docker's default 64 MB `/dev/shm` crashes renderers under load ("Aw, Snap"). brr also passes `--disable-dev-shm-usage` to Chromium inside containers, but a large `/dev/shm` is faster. `brr doctor` warns below 512 MB. |
| `-p 127.0.0.1:9898:9898` | brr has no authentication (ADR 0002). Anyone who can reach the port can launch Instances, run JavaScript in logged-in Profiles and use Credentials. **Never publish it beyond 127.0.0.1** unless a network boundary you control sits in front. |
| `-v brr-data:/data` | Profiles, Builds, the database and the vault key live under `/data`. |
| `--security-opt seccomp=...` | Sandbox policy A, below. |

Never use `--privileged` or `seccomp=unconfined` to make Chrome work.

## Sandbox policy

Chrome sandboxes its renderers with user namespaces, which Docker's default seccomp profile blocks without
`CAP_SYS_ADMIN` ("No usable sandbox"). Choose one:

- **A: seccomp profile (recommended).** `deploy/seccomp-chrome.json` is Docker's default profile plus an allow rule for
  `clone`, `clone3`, `unshare` and `setns`. Chrome keeps its sandbox. Regenerate it with `just docker-seccomp`
  (moby `docker-v29.9.2`).
- **B: `--no-sandbox`.** Omit the seccomp option and add `"--no-sandbox"` to the Profile's `args`. Isolation is
  weaker: a renderer exploit runs as the `brr` user inside the container. Use it where user namespaces are disabled on
  the host (for example Ubuntu's AppArmor restriction on unprivileged user namespaces).

The image never runs as root, because Chrome refuses to start as root without `--no-sandbox`.

## Data and the vault key

`BRR_VAULT__KEY_SOURCE=file`: the Credential vault key is a file in the volume (`/data/config`). Back up the whole
volume; without the key file the stored Credentials cannot be decrypted. Config, data and state are
`/data/config`, `/data/data` and `/data/state`; the socket and lock are in `/run/brr` (mode 0700).

## Builds inside the container

Debian's `chromium` is installed, so `POST /v1/builds/detect` (or `brr build detect`) registers it as a system Build on
both architectures and the container works offline. Chrome for Testing and Camoufox install on demand into
`/data`; the Camoufox runtime libraries are already in the image. The image carries no Rust toolchain.

## Checks

```sh
curl -s http://127.0.0.1:9898/v1/system/info
docker exec brr brr doctor        # Xvfb, /dev/shm size, Docker detection, user namespaces
```

`bash deploy/smoke-test.sh` runs the whole flow against `brr:dev`: policy A, then policy B. It starts and removes its
own containers.

## arm64

- Native image: `just docker-build --platform linux/arm64` (QEMU or an arm64 builder; buildx cannot `--load` a
  multi-platform build, so pass `--push` or `--output` for several platforms).
- Binary only: `just build-release-arm64` writes `target/aarch64-unknown-linux-gnu/release/brr`. It needs
  `gcc-aarch64-linux-gnu` (rusqlite's bundled SQLite and zstd compile C) and
  `rustup target add aarch64-unknown-linux-gnu`; it exits 2 with that hint otherwise.
- Build availability on arm64: Chrome for Testing publishes arm64 only from 153, so older pins fail with
  `BuildMissing`; fingerprint-chromium and Edge are unavailable; ungoogled-chromium portable, Camoufox and Debian's
  chromium work.

## Licenses

The image ships `/usr/share/doc/brr/`: `NOTICE`, `autoconsent/` (the vendored MPL-2.0 bundle, its LICENSE and version, which
is the source offer) and `playwright/` (Playwright's LICENSE and NOTICE for the injected script). The companion
extension is in `/usr/share/brr/extension`.
