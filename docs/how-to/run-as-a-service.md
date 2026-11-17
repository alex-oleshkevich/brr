# Run brr as a service

By default the first `brr` command starts the daemon in the background, and it runs until you stop it or the machine restarts. To start it with your session and restart it after a crash, install it as a systemd user service.

## Install the service

```sh
brr daemon install
```

This writes `~/.config/systemd/user/brr.service`, enables it and starts the daemon under systemd. A daemon you started by hand is stopped first, so systemd owns the new one. The unit restarts the daemon if it fails.

The unit passes `PATH`, `DISPLAY`, `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR` and the XDG config, data and state directories to the daemon. It does not pass `BRR_*` variables, so put settings in the config file with `brr settings set` instead (see [Configuration](../reference/configuration.md)).

The service starts when you log in. To keep it running when you are logged out, as on a server, enable lingering for your user:

```sh
loginctl enable-linger "$USER"
```

## Check on it

```sh
brr daemon status          # version, memory, browsers running
brr daemon logs            # last 100 lines of the daemon log
brr daemon logs -n 500     # more lines
```

The log file is `~/.local/state/brr/daemon.log`. `systemctl --user status brr` also works.

## Upgrade

Run the install script again, then restart the daemon so it runs the new binary:

```sh
curl -fsSL https://raw.githubusercontent.com/alex-oleshkevich/brr/main/install.sh | sh
brr daemon stop && brr daemon start
```

If you built from source, run `just install` instead of the script. `brr daemon start` goes through systemd when the service is installed.

## Remove the service

```sh
brr daemon uninstall
```

This stops the daemon, disables the unit and deletes the unit file. Your profiles, builds and settings stay. The next `brr` command starts the daemon in the background again.
