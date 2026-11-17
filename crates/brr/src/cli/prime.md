# brr agent guide

brr runs real browsers for you. A **Profile** is a saved browser identity (cookies, logins,
settings). Launching it starts an **Instance** (one running browser) with **Tabs**. You hold a
**Lease** on each Tab you open, so no other agent can drive it until you let go.

## Pick the surface

- **MCP tools** (`profile_launch`, `navigate`, `snapshot`, …) when your host has brr connected.
  Every tab tool takes an optional `tab_id`, which defaults to your current Tab: the last one you
  opened or switched to.
- **CLI** `brr …` otherwise. Each command is its own Client, so nothing carries over between
  commands: pass `--tab-id` to every tab command and `--json` when you parse the output. The CLI
  starts the daemon if it is down.
- **A one-off read** of a public page needs no Profile: `oneshot_markdown{url}`, or
  `brr shot markdown <url>`. Also `screenshot`, `pdf`, `content`, `links`.

## The loop

1. **Launch.** `profile_launch{profile, url}`. It returns the Instance, its first Tab and your
   Lease. With no Profile yet, create one from a template:
   `profile_create{name, template: "chromium-headless"}`.
2. **Look.** `snapshot` returns the page's accessibility tree with a `ref` on each element. Use
   `get_markdown` to read content, and `screenshot` only for layout or visuals.
3. **Act** by ref: `click{ref}`, `fill{ref, value}`, `press_key{key}`, `select_option`. Use
   `navigate{url}` for a new page and `tab_open{url}` for a second Tab.
4. **Look again** after every action that changes the page. Refs go stale after the next snapshot
   or navigation (`StaleRef`).
5. **Stop** what you launched: `instance_stop{instance_id}`. Ephemeral Profiles lose their data
   on stop; Persistent ones keep it. An idle browser is reaped anyway.

| Profile templates | What you get |
|---|---|
| `chromium-headless` | Chrome without a window. The default for agent work. |
| `chromium-visible` | Chrome with a window the user can watch. |
| `camoufox-stealth` | Firefox-based browser that hides automation, on a virtual screen. Use it for sites that block bots. |
| `camoufox-headless` | The stealth browser with no window. |

## Batching

`batch_run{steps: [{tool, args}, …]}` runs Browser tools in order in one call, so a flow you can
plan up front costs one turn instead of one per step. It runs blind: nothing is looked at between
steps. Use it only when you know every step in advance, such as filling a form and submitting it.

- **Use `selector`, not `ref`.** Refs go stale after any navigation or page change, so a `ref`
  from before the batch fails with `StaleRef` as soon as a step changes the page.
- **Pass `tab_id` on every step.** Without a `client_id`, `tab_open` does not set a current Tab for
  the steps after it.
- **It stops at the first error** unless `continue_on_error` is set, or after about two minutes
  between steps. Steps that already ran stay applied; steps after the stop are not run and not
  listed. Each entry in `results` has its `step` index; `completed` counts the steps that ran
  (failed ones included) and `failed` the ones that failed. Compare `completed` with `total`, then
  take a `snapshot` before you continue.
- **Do not batch** a step whose input comes from an earlier step's output, anything that may hit a
  captcha or login wall, or exploratory browsing. Make those separate calls.
- Arguments are checked when each step runs, not up front, so a typo in step 5 fails at step 5
  after steps 1 to 4 ran. Only the tool names are checked before step 1. A rejected request comes
  back as a tool error; a failed step comes back as a result with `ok: false`.

## People and secrets

- **Logins.** Call `credential_list`, then `login{credential_id}`. The daemon types the password
  and the one-time code, so you never see them. Never ask the user to paste a password into chat.
- **Captchas, 2FA prompts, consent walls you cannot pass.** Call `await_human{reason: "captcha" |
  "login" | "dialog"}`. The user gets a notification and finishes it in the brr web UI. The call
  returns once they are done, or fails with `Timeout`. Tell the user what you are waiting for.
- **Someone else's Tabs.** Tabs leased to another Client are read-only for you: snapshot and read
  them, but leave them alone. `LeaseHeld` names the holder.

## When a call fails

Every error says what to do. The ones that change your plan:

| Error | Do |
|---|---|
| `StaleRef`, `ElementNotFound` | Take a new `snapshot` and pick the element again. |
| `LeaseHeld` | Another Client owns the Tab or the Instance. Open your own Tab, or wait. |
| `LeaseLost` | The user took over your Tab or the Lease was released. Stop driving it and check `tab_get`. |
| `BuildMissing` | Install the browser: `build_install{source, channel}`, poll `build_get` until it is `installed`, then launch again. |
| `CapExceeded` | The machine is full. Stop an Instance you no longer need, then launch again. |
| `Conflict` with reason `profile_locked` | The Profile already runs. Find it with `instance_list` and open a Tab in it with `tab_open{instance_id}`, or use another Profile. |

## More

- `brr <command> --help` lists every flag. MCP tool schemas describe every argument.
- `brr doctor` checks this machine. `events_list` shows what happened, newest first.
- The web UI shows the browsers live and lets the user step in. Its address is below.
