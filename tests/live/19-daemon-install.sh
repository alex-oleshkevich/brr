#!/usr/bin/env bash
# §15: `brr daemon install` writes a systemd user unit; systemctl is faked, the real one is never called.
source "$(dirname "$0")/lib.sh"
set -euo pipefail

fake=$SCRATCH/fake
mkdir -p "$fake"
calls=$SCRATCH/systemctl.log
cat >"$fake/systemctl" <<SH
#!/usr/bin/env bash
echo "\$*" >>"$calls"
# Like systemd, \`start\` leaves a daemon running; the harness stops it through this pid file.
if [[ "\$*" == *"start brr"* ]] && ((\${FAKE_SYSTEMCTL_EXIT:-0} == 0)); then
    (setsid "$BRR_BIN" daemon run 2>>"$XDG_STATE_HOME/brr/daemon.log" >/dev/null & echo \$! >"$SCRATCH/daemon.pid")
fi
exit \${FAKE_SYSTEMCTL_EXIT:-0}
SH
chmod +x "$fake/systemctl"
export PATH=$fake:$PATH
unit=$XDG_CONFIG_HOME/systemd/user/brr.service

brr daemon install >/dev/null
CHECKS=$((CHECKS + 1))
[[ -f $unit ]] || fail "no unit at $unit"
grep -q '^PassEnvironment=.*DISPLAY' "$unit" && grep -q 'WAYLAND_DISPLAY' "$unit" && grep -q 'XDG_RUNTIME_DIR' "$unit" ||
    fail "unit lacks PassEnvironment=DISPLAY WAYLAND_DISPLAY XDG_RUNTIME_DIR: $(<"$unit")"
CHECKS=$((CHECKS + 1))
grep -q "^ExecStart=.*brr.* daemon run" "$unit" || fail "unit lacks ExecStart: $(<"$unit")"
for want in 'daemon-reload' 'enable brr' 'start brr'; do
    CHECKS=$((CHECKS + 1))
    grep -q -- "$want" "$calls" || fail "systemctl was not called with '$want': $(<"$calls")"
done

: >"$calls"
brr daemon uninstall >/dev/null
CHECKS=$((CHECKS + 1))
[[ ! -e $unit ]] || fail "unit survives uninstall"
CHECKS=$((CHECKS + 1))
grep -q -- 'disable --now brr' "$calls" || fail "uninstall did not call 'disable --now brr': $(<"$calls")"

# A failing systemctl fails the install and names the command.
rc=0
err=$(FAKE_SYSTEMCTL_EXIT=1 brr daemon install 2>&1 >/dev/null) || rc=$?
CHECKS=$((CHECKS + 1))
((rc != 0)) || fail "install succeeded although systemctl exits 1"
CHECKS=$((CHECKS + 1))
[[ $err == *systemctl* ]] || fail "stderr does not name systemctl: $err"

finish "unit written and removed through a fake systemctl"
