# Engines: Chromium family and Camoufox only, no stock Firefox

Supported engines are the Chromium family (driven over CDP) and Camoufox (a patched Firefox driven over Playwright's Juggler protocol). Stock Firefox is excluded. Playwright cannot drive stock Firefox, and Firefox's only remote protocol now is WebDriver BiDi, which allows one client at a time. Supporting it would mean a third protocol stack for a browser with no stealth value over Camoufox. Firefox WebExtensions also lack a debugger API (Mozilla bug 1840132), so a Live browser extension for Firefox could not control tabs.
