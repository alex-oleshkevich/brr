# Save logins

brr keeps site logins in an encrypted vault and links each one to the profiles that may use it. When an agent asks brr to log in, the daemon types the username, password and 2FA code into the page itself. The agent gets the result but never sees the secrets, and `brr credential list` never returns them.

## Store a login

```sh
read -rs PW
brr credential create https://shop.example.com --username alice@example.com --password "$PW" --profile-ids my-first
```

`read -rs` asks for the password without echoing it and keeps it out of your shell history.

- `--profile-ids` takes a profile name or id. Repeat it to link several profiles.
- `--totp` takes the 2FA secret, as base32 (`JBSWY3DPEHPK3PXP`) or an `otpauth://` URI, so brr can type the current code.
- `--login-url` sets the page to log in on, when it is not the site's address.
- `--selectors` tells brr which fields to fill when it cannot find them itself, as JSON with any of `username_selector`, `password_selector`, `totp_selector` and `submit_selector`:

  ```sh
  --selectors '{"username_selector": "#email", "password_selector": "#pass"}'
  ```

List what is stored:

```sh
brr credential list --json | jq -r '.credentials[] | "\(.id) \(.site) \(.username)"'
```

Change a login with `brr credential update <id>` and the flags you want to change, for example `--password`. Delete it with `brr credential delete <id>`.

## Import from Chrome or Firefox

Export your passwords from the browser as CSV, then:

```sh
brr credential import passwords.csv --format chrome --profile-ids my-first
```

Use `--format firefox` for a Firefox export. brr reports how many logins it imported and how many it skipped because they were already in the vault. Delete the CSV afterwards; it holds every password in plain text.

## Log in from a tab

Launch a profile that is linked to the login, then:

```sh
brr tab login <credential id> --tab-id "$TAB"
```

```
ok     true
steps  1
url    https://shop.example.com/account
```

brr loads the login page in the tab (the `--login-url`, or the site), fills the form and submits it. Where a secret would appear in the output, such as a URL, brr prints `«redacted»`. Over MCP the same call is the `login` tool.

## Use Bitwarden or Vaultwarden

brr can type logins that stay in your Bitwarden vault. It fetches the secret when the login runs and does not copy it into its own database.

Connect with the personal API key from your Bitwarden web vault and your master password:

```sh
brr bitwarden connect --client-id user.xxxxxxxx --client-secret xxxxxxxx --master-password "$BW_PASSWORD"
```

For the EU cloud or a self-hosted Vaultwarden, add `--server-url https://vault.bitwarden.eu` or your server's address. The address must use https; plain http is allowed only for localhost. brr keeps the key and password only if the vault opens.

Find an item and link it to a profile:

```sh
brr bitwarden items shop
brr bitwarden link <item id> --profile-ids my-first
```

The linked item becomes a credential like any other; use its id with `brr tab login`. brr syncs the vault every 15 minutes (`bitwarden.sync_interval_min`). To sync now, or to see the state:

```sh
brr bitwarden sync
brr bitwarden status
```

`brr bitwarden disconnect` forgets the key and password. Linked credentials stay but fail as locked until you connect again.

## Check that a profile is still logged in

A login check is a page plus a selector or text that appears only when you are logged in:

```sh
brr login-check create my-first --site https://shop.example.com --url https://shop.example.com/account --expect-text "Sign out"
```

brr runs it on the profile's running browser every 60 minutes (`login_check.interval_min`), or now with `brr login-check run <id>`. Add `--credential-id <id> --auto-relogin` to log in again when the check fails.
