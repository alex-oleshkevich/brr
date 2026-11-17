# Run browsers on another machine

A Host is another machine that runs brr's browsers in Docker. brr reaches it with `docker -H ssh://...`; you install nothing of brr there. You keep one daemon, one web UI and one API, and brr places each launch on the machine with room and the most free memory.

## Before you start

- [Docker isolation](isolate-browsers-in-docker.md) is on: `sandbox.mode` is `docker`. Remote browsers always run in containers.
- Docker is installed on the Host, and the SSH user can run it.
- The user that runs the brr daemon can run `ssh user@host docker version` without a password prompt. brr uses that user's SSH keys or agent and stores no SSH secrets.

## Add the Host

```sh
brr host create build-box --docker-url ssh://alex@build-box.example.com --max-instances 4
```

`--max-instances` is how many browsers may run there at once. Add `:port` to the URL if SSH is not on port 22.

brr checks the Host as soon as you add it. The `status` field shows whether Docker answered, the Host's memory and whether the brr image is there. If the check fails, the error says what went wrong, for example:

```
docker on ssh://alex@build-box.example.com did not answer: ... check that the daemon's user can run `ssh <user@host> docker version` without a prompt
```

Fix the problem and check again:

```sh
brr host check build-box
```

## Launch browsers

Launch profiles as usual. brr picks the machine for each launch: its own Docker or any enabled Host that has room, preferring the one with the most free memory.

The first launch on a Host copies the brr image and the browser build to it, which takes about a minute. Later launches reuse them.

A remote run copies the profile to the Host before the browser starts and copies it back after the browser stops. If the connection drops during a run, the run ends and the profile keeps the copy it had before the run. All browser traffic goes back through brr, so proxies, their credentials and traffic counts stay on brr's machine.

To see where a browser runs, look at `host_id` in `brr instance get <id>`. It is `null` for brr's own machine.

## Manage Hosts

```sh
brr host list
brr host get build-box
brr host update build-box --max-instances 8
brr host update build-box --enabled=false     # stop placing new browsers there
brr host delete build-box                     # refused while it runs browsers
```
