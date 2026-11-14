# brr companion extension

Chrome MV3 extension (Chromium family only) that lets a brr daemon control tabs in your everyday browser through `chrome.debugger`.

## Install

- Build: `just ext-build`, then load `extension/dist` as an unpacked extension (`chrome://extensions`, Developer mode).
- Or unzip `brr extension companion -o brr-companion.zip` and load that directory; it contains `brr.json` with the daemon URL.

## Consent flow

Nothing connects on its own. Click the toolbar icon, check the daemon URL and press "Allow and connect". The extension adds the most recently active http(s) tab of the current window to a blue "brr" tab group, opens the WebSocket to the daemon and shares the tabs in that group. Drag more tabs into the group to share them; drag them out or close them to stop. Every new connection (after a disconnect or daemon restart) asks again.

The popup shows the URL from `brr.json` when present, otherwise the last URL used, otherwise `ws://127.0.0.1:9898/v1/extension-relay`.

## Limits

- Shared tabs show Chrome's "being debugged" bar; closing it detaches that tab.
- `chrome://` and `devtools://` pages cannot be controlled, even inside the group.
- Opening DevTools on a shared tab detaches it.
- Tabs outside the brr group are never visible to the daemon.

## Develop

`just ext-test` runs the unit tests. Set `BRR_TEST_CHROME` to a Chrome for Testing binary to also run `browser.test.ts` (needs `just ext-build` first).
