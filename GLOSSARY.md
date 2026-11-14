# Browser Manager

A daemon that installs browsers, runs them from profiles, tracks their tabs, closes what sits idle, and limits how many run at once. People and agents control it through a web UI, MCP and a CLI.

## Browsers

**Engine**:
A browser family the manager can run: the Chromium family or Camoufox.
_Avoid_: browser type, flavor

**Build**:
One installed copy of an engine at a specific version.
_Avoid_: binary, installation

**Build install**:
Fetching a Build onto the machine.
_Avoid_: download (means a file saved by a Tab)

**Capability**:
Something a Build can do (PDF export, live screencast, unpacked extensions, …). A tool that needs a Capability the Instance's Build lacks fails with `EngineUnsupported`.
_Avoid_: feature, support flag

**Channel**:
A moving version line a Profile can follow instead of pinning a Build (`stable`, `latest-compatible`).
_Avoid_: release track

## Profiles

**Profile**:
A named set of browser settings (engine, proxy, fingerprint, locale, extensions, limits) that Instances are launched from.
_Avoid_: config, identity, session

**Persistent profile**:
A Profile that owns one lasting set of browser data (cookies, bookmarks, history). At most one Instance of it runs at a time.

**Ephemeral profile**:
A Profile whose every launch starts with fresh browser data that is discarded when the Instance stops. Any number of its Instances may run at once.
_Avoid_: temporary profile, incognito

**Promote**:
Saving an ephemeral Instance's browser data as a new Persistent profile.

**Profile template**:
A named starting point for new Profiles. Templates are built in (e.g. camoufox-stealth) or saved by the user. A Profile copies its template's settings when created and does not follow later changes to it.
_Avoid_: preset, default profile

**Credential**:
A stored site login (username, password, optional 2FA secret) linked to one or more Profiles. Agents can use it to log in but never read it.
_Avoid_: password, secret, account

## Running browsers

**Instance**:
One running browser launched from a Profile, or one Live browser attached by the manager.
_Avoid_: session, browser process, context

**Live browser**:
A browser the user started themselves that the manager attaches to. The manager never stops it: when the user closes it, its Instance becomes lost; detaching releases it.
_Avoid_: external browser, user browser

**Tab**:
One page inside an Instance.
_Avoid_: page, target

**Browser context**:
An isolated cookie and storage partition inside one Instance.
_Avoid_: incognito, session

**Current Tab**:
The Tab a Client's Tab tools act on when no Tab is named.

**Element ref**:
An id from the latest snapshot of a Tab that names one element.

**Client**:
Whoever acts through the manager: an MCP client, a CLI call, a web UI page, a REST caller, or an outside program connected to an Instance's endpoint. A Client ends when it disconnects, says so, or goes quiet too long.
_Avoid_: user, consumer, agent

**Lease**:
A Client's claim on an Instance or Tab. Idleness is measured from the last activity under a Lease.
_Avoid_: claim, lock, ownership

**Activity**:
An operation under a Lease, a frame on a connect WebSocket, or human input through live view. Page network traffic and timers are not Activity.
_Avoid_: traffic, usage

**Pinned**:
Marked as never closed for idleness. Applies to a Profile or a Tab.

**Reaping**:
The manager closing a Tab or Instance because it has been idle too long, or to free resources.
_Avoid_: cleanup, garbage collection

**Reaper**:
The part of the manager that performs Reaping.

**Grace period**:
The time an Instance or Tab spends in reaping before it is closed; any Activity cancels it.

**Eviction**:
Reaping an idle Instance immediately, without a Grace period, to admit a waiting launch.

## Capacity

**Limit mode**:
How the manager decides whether another Instance or Tab may start: **static** (fixed counts) or **adaptive** (free memory and CPU load, still bounded by the fixed counts).
_Avoid_: capping mode, throttle

**Admission**:
The decision to start, queue, reject, or make room for a requested launch when a limit is reached.

**Admission queue**:
Launches waiting for capacity, ordered by priority then arrival.

**One-shot request**:
A call that opens a URL, returns one result and closes its Browser context.

**Ready pool**:
Pre-started one-shot Instances waiting for One-shot requests.

## Storage

**Storage backend**:
Where the manager keeps everything that outlives an Instance: local disk (default), an S3-compatible bucket, or Cloudflare R2.
_Avoid_: bucket, remote, cloud storage

**Artifact**:
A stored output of a Tab: a Download, or a capture result that was stored rather than returned inline (screenshot, PDF, MHTML, HAR). Every Artifact is tracked so a Retention policy can sweep it. Profile backups are not Artifacts.
_Avoid_: file, output, result

**Download**:
An Artifact that a Tab saved through the browser's own download.
_Avoid_: file

**Retention policy**:
Rules for how long and how much stored data (Artifacts, ephemeral leftovers, unused Builds, event history) is kept before the manager deletes it, set globally and per Profile.
_Avoid_: cleanup rules, TTL

## Network

**Proxy pool**:
A named set of proxies handed out sticky per Profile or rotated per launch.

**Forwarding proxy**:
The manager's local proxy that adds upstream credentials for a browser.

## Logins

**Login check**:
A URL plus an element or text that proves a Profile is logged in to a site.

**Companion extension**:
brr's browser extension that lets the manager drive Live browser tabs after the user consents.

## Captcha

**Captcha policy**:
A Profile's rule for captchas: off, on-demand, auto, or human.

**Human step-in**:
A person solving a captcha or login in a Tab through the web UI's live view.

## UI words

The web UI never shows the internal names. API, CLI, MCP, specs and event kinds keep theirs; only user-visible text changes, and every page uses the same word.

| Internal | UI word |
|---|---|
| Instance | **browser** ("running browser" where needed); nav item "Browsers" |
| Lease, held by | **"In use by <agent label>"**, or "You're in control" when the UI holds it. Never "Lease" |
| Client | **agent** (its label). The UI's own client is "you" |
| Build | **browser version**; nav item "Browser versions" |
| Artifact | **file**; nav item "Files" |
| Events | **Activity** |
| Limits, Admission | **Capacity**; "Waiting to start" replaces "admission queue" |
| Static / Adaptive limit mode | **"Fixed limits" / "Based on free memory"** |
| One-shot pool, linger | "Pre-started browsers for one-off jobs"; "Keep a one-off browser open after its job (s)" |
| Reaping grace | "Warning period before closing an idle browser (s)" |
| Step-in, await_human | **"Step in"**; banner "<agent> needs you to <reason> on <host>" |
| Take over, Release, Hand back, Done | **"Take control"**; **"Hand back to <agent>"** when an agent is waiting or was in control, otherwise **"Release"**; **"Done — hand back to <agent>"** |
| Capability chips | One line, e.g. "Standard · no PDF" |
| Template | "Profile template" with a plain title and a one-line purpose |
| RSS | Memory |
| Credentials | **Logins** |

Nav groups: **Run** (Dashboard, Browsers, Activity, Files, Connect an agent), **Set up** (Profiles, Proxies, Logins, Extensions), **System** (Browser versions, Captcha, Capacity, Storage, Settings).

Words like "tab" and "browser" are not capitalised as proper nouns in copy.
