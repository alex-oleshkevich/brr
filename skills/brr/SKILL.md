---
name: brr
description: Use for browsing with a real browser: open sites, read or screenshot pages, fill forms, log in, scrape, or anything behind a login, captcha or bot check.
---

# brr

brr runs real browsers from saved Profiles and lets a person step in when you are stuck.

**Run `brr prime` first.** It prints the guide, plus what is running right now: installed
browsers, Profiles, open browsers and the web UI address. Run it again after your context is
cleared or compacted.

When `brr prime` is unavailable, here is the core loop. MCP tool names are shown first, with the
CLI command in brackets:

1. `profile_launch{profile, url}` (`brr profile launch <profile> --url <url>`). With no Profile
   yet, create one: `profile_create{name, template: "chromium-headless"}`.
2. `snapshot` to get element refs, then `get_markdown` to read the page.
3. `click{ref}`, `fill{ref, value}`, `navigate{url}`. Take a new snapshot after each change,
   because refs go stale.
4. Stuck on a captcha or login: `login{credential_id}` with a saved login, or
   `await_human{reason}` so the user finishes it in the brr web UI. Never ask for a password in
   chat.
5. `instance_stop{instance_id}` when done.

A one-off read of a public page: `brr shot markdown <url>`.

`batch_run{steps: [{tool, args}]}` runs several Browser tools in one call. Use it only for steps
you can plan without seeing the page in between, address elements by `selector` (refs go stale),
pass `tab_id` on every step, and take a `snapshot` after it stops on an error.
