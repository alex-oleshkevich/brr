# Use a proxy

A profile can send its traffic through one proxy or through a pool of proxies. brr runs a small local forwarder for each browser that adds the proxy's login, so the password never reaches the browser.

## Add a proxy

```sh
brr proxy create --scheme http --host proxy.example.com --port 8080 --username me --password secret
```

`--scheme` is `http`, `https` or `socks5`. Leave out `--username` and `--password` for a proxy without a login.

brr checks the proxy when you add it and every 60 minutes after that. Run a check yourself to see the result:

```sh
brr proxy list --json | jq -r '.proxies[] | "\(.id) \(.host):\(.port)"'
brr proxy check <proxy id>
```

```
exit_ip     203.0.113.7
healthy     true
latency_ms  194
```

## Add many proxies at once

Put one proxy per line in a file, in any of these forms:

```
proxy1.example.com:8080
proxy2.example.com:8080:user:pass
socks5://user:pass@proxy3.example.com:1080
```

```sh
brr proxy import --scheme http -i proxies.txt
```

`--scheme` applies to lines that do not name one. brr reports how many it imported, how many it skipped as duplicates and which lines it could not read. IPv6 addresses cannot be imported; add them with `brr proxy create`.

## Use it in a profile

A profile's `proxy` setting holds `{"proxy_id": "..."}` or `{"pool_id": "..."}`. `brr profile update --settings` replaces all of a profile's settings, so read them, change the proxy and write them back:

```sh
PROXY=<proxy id>
brr profile update my-first --settings "$(brr profile get my-first --json | jq -c --arg p "$PROXY" '.settings | .proxy = {proxy_id: $p}')"
```

The next launch of `my-first` goes through the proxy; its instance record shows the `proxy_id`. To remove the proxy, set `.proxy = null` the same way.

## Rotate through a pool

A pool hands out its proxies either `sticky` (each profile keeps the same proxy) or `rotate` (a different proxy on each launch). Repeat `--proxy-ids` for each member:

```sh
brr pool create shop-pool --mode rotate --proxy-ids <id 1> --proxy-ids <id 2>
POOL=$(brr pool list --json | jq -r '.pools[] | select(.name == "shop-pool") | .id')
brr profile update my-first --settings "$(brr profile get my-first --json | jq -c --arg p "$POOL" '.settings | .proxy = {pool_id: $p}')"
```

## See how much traffic went through

```sh
brr usage traffic --group profile
brr usage traffic --group proxy --days 30
```

Each item shows bytes sent and received and the number of runs. brr counts the traffic that passes through its forwarder: runs with a proxy, and every run in the [Docker sandbox](isolate-browsers-in-docker.md). Other runs are not measured.

## Remove proxies

```sh
brr pool delete shop-pool
brr proxy delete <proxy id>
```

brr refuses to delete a proxy or pool that a profile names in its `proxy` setting. Deleting a proxy removes it from any pool it belongs to.
