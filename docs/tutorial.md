# Your first browser

In this tutorial you install a browser with brr, start it from a profile, read a web page through it, take a screenshot, look at it in the web UI and then stop it. It takes about five minutes, most of it waiting for the browser download.

You need `brr` installed (see the [README](../README.md#install)) and `jq` for one step.

## 1. Start the daemon

brr is a background daemon that the `brr` command talks to. Start it:

```sh
brr daemon start
```

```
     Started daemon
```

Any other `brr` command would also start it, but starting it yourself makes the first step visible.

## 2. Check the machine

```sh
brr doctor
```

You get a table of checks. On a new machine one of them fails:

```
    chromium_build   fail  no Chromium-family Build; run brr build detect or brr build install --source cft
```

That is expected: brr has no browser yet. The next step installs one.

## 3. Install a browser

Install Chrome for Testing, the build of Chrome that Google publishes for automation:

```sh
brr build install --source cft
```

The command returns at once with `state  installing` while the download runs. Check on it:

```sh
brr build list
```

```
builds:
    Chrome for Testing  01M7NBJ7D4MQ0XCZW7K4FVBFT4  chromium  installed  cft  157.0.8103.78
```

Run `brr build list` again until the line says `installed`. Your id and version will differ.

## 4. Create a profile

A profile is a named set of browser settings plus the browser's data (cookies, history). Create one from the `chromium-headless` template, which runs Chrome without a window:

```sh
brr profile create my-first --template chromium-headless
```

brr prints the new profile, including `name  my-first` and `kind  persistent`. A persistent profile keeps its cookies from one run to the next.

## 5. Launch it

```sh
brr profile launch my-first --url https://example.com
```

brr starts Chrome and opens https://example.com. It prints three records: the `instance` (the running browser), the `tab` it opened and your `lease` on that tab.

List the open tabs:

```sh
brr tab list
```

```
tabs:
    01M7NBVMKF44AM0TJ1HNPB6MA3  ready  https://example.com/
```

## 6. Read the page

Every `brr` command is separate, so each tab command needs the tab id. Save it in a shell variable:

```sh
TAB=$(brr tab list --json | jq -r '.tabs[0].id')
```

`--json` makes any command print one line of JSON, which `jq` can pick apart. Now read the page as Markdown:

```sh
brr tab get-markdown --tab-id "$TAB"
```

```
# Example Domain

This domain is for use in documentation examples without needing permission. ...
```

## 7. Take a screenshot

```sh
brr tab screenshot --tab-id "$TAB" -o example.png
```

```
       Wrote example.png (39385 bytes)
```

Open `example.png` to see the page as Chrome rendered it.

## 8. Look at it in the web UI

Open http://127.0.0.1:9898 in your own browser. The Browsers page lists the browser you launched, with its tab on example.com. Leave the page open for the next step.

## 9. Stop the browser

Get the id of the running browser and stop it:

```sh
INSTANCE=$(brr instance list --state running --json | jq -r '.instances[0].id')
brr instance stop "$INSTANCE"
```

brr prints the instance again, now with `state  stopped` and `end_reason  stop`. The web UI shows that it stopped. If you forget to stop a browser, brr closes it once it has sat idle for a while (the `idle.*` settings in the [configuration reference](reference/configuration.md)).

## What you did

You installed a browser, created a profile, launched it, read and captured a page, and stopped it. Launch `my-first` again and it still has the cookies from this run.

Next:

- [Connect an AI agent](how-to/connect-an-ai-agent.md) so your coding agent can do the same through MCP.
- [Concepts](explanation/concepts.md) explains how builds, profiles, instances, tabs and leases fit together.
- [CLI reference](reference/cli.md) lists every command group.
