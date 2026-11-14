# No authentication and no request-origin checks

The daemon binds 127.0.0.1:9898 by default; the bind address and port are configurable. No surface (web UI, REST, MCP, Instance endpoints) requires authentication, and the daemon does not check Host or Origin headers. This was chosen deliberately, despite a known risk: any web page open on the machine can send requests to the daemon, and so can anything on the network when it binds a non-loopback address. Such requests can launch Instances, run JavaScript in logged-in Profiles and trigger logins from the Credential vault. Do not add these checks without revisiting this decision.

## Exception: the companion relay checks Origin

`WS /v1/extension-relay` requires the handshake `Origin` to start with `chrome-extension://` or `moz-extension://`; any other Origin, or none, gets 403 before the upgrade. Without this, a web page open in the user's browser could connect to the relay, present itself as the companion and become a Live Instance, and an agent calling `instance.attach {via:"extension"}` would then send its CDP traffic to that page. A web page cannot forge an extension Origin, so this is the narrowest check that closes the hole. It applies to this one route only: every other surface keeps the decision above. The companion has no manifest key yet, hence no stable extension ID to pin; add the pin when it gets one.

## Consequence: the local proxy forwarder

The per-Instance forwarding proxy binds an ephemeral port on 127.0.0.1 and adds the upstream proxy's credentials to whatever connects, so any local user or process can route traffic through a configured proxy and its exit IP. This is accepted under the same decision: a host where other local users are not trusted is outside the model.
