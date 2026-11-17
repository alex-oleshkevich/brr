# Browser sandbox: none or docker

Managed browsers run either as today (`sandbox.mode = none`) or each in its own Docker container
(`docker`). In a container a `brr runner` process starts the browser on fresh fd 3/4 socketpairs
and relays them over the container's stdin/stdout, so brr still owns the only browser connection
([ADR 0004](0004-one-pipe-connection-per-instance.md)) and no other runtime is added
([ADR 0003](0003-pure-rust-no-sidecar.md)). The container has no network; the browser reaches the
internet only through brr's forwarder over a Unix socket, so upstream proxy credentials never enter
it, and the forwarder refuses loopback, private and link-local destinations. Inside, the browser
runs with `--no-sandbox`; the container is the boundary.

We chose Docker over Landlock and bubblewrap: bubblewrap and Chromium's own sandbox need
unprivileged user namespaces, which hardened hosts block, and Landlock on current LTS kernels cannot
filter UDP or signals. Docker gives full isolation with resource limits on every host we deploy to.

## Consequences

- brr holds the Docker socket, so a compromise of brr is a compromise of the host. brr must stay
  behind authentication when exposed (ADR 0002 still holds for the daemon itself).
- Visible display is not available in docker mode. Live (attached) browsers are never sandboxed.
- Do not pass upstream proxy credentials into a container, or give it a network route, without
  revisiting this decision.
