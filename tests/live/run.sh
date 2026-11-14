#!/usr/bin/env bash
# Live tests: one scratch daemon per script. See tests/live/README.md.
# Usage: tests/live/run.sh [name...]   (name: 05, instances-tabs or 05-instances-tabs)
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
cd "$ROOT" || exit 1

for tool in jq curl node; do
    command -v "$tool" >/dev/null || { echo "FAIL setup $tool is required"; exit 1; }
done

source "$HERE/lib.sh"

SCRIPT_TIMEOUT_S=900
ORIG_PATH=$PATH
HOST_RUNTIME_DIR=${XDG_RUNTIME_DIR-}
SCRATCH=
FAILED=0

if [[ -z ${BRR_LIVE_NO_BUILD-} ]]; then
    just build >&2 || { echo "FAIL build just build failed"; exit 1; }
fi
BRR_BIN=$ROOT/target/debug/brr
export BRR_BIN

selected() {
    local file=$1 base arg
    base=$(basename "$file" .sh)
    ((${#NAMES[@]} == 0)) && return 0
    for arg in "${NAMES[@]}"; do
        arg=${arg%.sh}
        [[ $arg == "$base" || $arg == "${base%%-*}" || $arg == "${base#*-}" ]] && return 0
    done
    return 1
}

# Anything a browser of this scratch run left behind: env carries an instance id and our state dir.
leaked_pids() {
    local env_file pid env_text
    for env_file in /proc/[0-9]*/environ; do
        [[ -O $env_file ]] || continue
        env_text=$(cat "$env_file" 2>/dev/null | tr '\0' '\n') || continue
        if grep -q '^BRR_INSTANCE_ID=' <<<"$env_text" && grep -qxF "XDG_STATE_HOME=$XDG_STATE_HOME" <<<"$env_text"; then
            pid=${env_file#/proc/}
            echo "${pid%/environ}"
        fi
    done
}

teardown() {
    [[ -n $SCRATCH && -d $SCRATCH ]] || return 0
    TEARDOWN_NOTE=
    stop_daemon 30 || TEARDOWN_NOTE="daemon ignored SIGTERM"
    local leaked
    leaked=$(leaked_pids)
    if [[ -n $leaked ]]; then
        TEARDOWN_NOTE="leaked browser pids: $(echo $leaked)"
        kill -KILL $leaked 2>/dev/null || true
    fi
    if [[ -n ${BRR_LIVE_KEEP-} ]]; then
        echo "kept $SCRATCH" >&2
    else
        rm -rf "$SCRATCH"
    fi
    SCRATCH=
}

abort() {
    teardown
    exit 130
}
trap abort INT TERM
trap teardown EXIT

run_one() {
    local file=$1 name log rc reason pin
    local -a pinned=()
    name=$(basename "$file" .sh)
    mkdir -p "${BRR_LIVE_TMPDIR:-/tmp}"
    SCRATCH=$(mktemp -d "${BRR_LIVE_TMPDIR:-/tmp}/brr-live.XXXXXX")
    mkdir -p "$SCRATCH"/{config,data,state,runtime,bin}
    log=$SCRATCH/script.log

    export XDG_CONFIG_HOME=$SCRATCH/config XDG_DATA_HOME=$SCRATCH/data
    export XDG_STATE_HOME=$SCRATCH/state XDG_RUNTIME_DIR=$SCRATCH/runtime
    export SCRATCH
    BIND=127.0.0.1:$(free_port)
    export BIND BRR_SERVER__BIND=$BIND BRR_VAULT__KEY_SOURCE=file
    export SOCK=$XDG_RUNTIME_DIR/brr/brr.sock

    # A visible browser needs the host's Wayland socket, which the scratch runtime dir does not hold.
    if [[ -n ${WAYLAND_DISPLAY-} && -S $HOST_RUNTIME_DIR/$WAYLAND_DISPLAY ]]; then
        ln -s "$HOST_RUNTIME_DIR/$WAYLAND_DISPLAY" "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
    fi

    PATH=$ORIG_PATH
    if [[ -n ${BRR_TEST_CHROME-} && -x ${BRR_TEST_CHROME-} ]]; then
        ln -s "$BRR_TEST_CHROME" "$SCRATCH/bin/google-chrome-stable"
        PATH=$SCRATCH/bin:$ORIG_PATH
    fi
    export PATH

    # A script may pin settings that must come from the environment: `# env: BRR_X__Y=1`.
    # Drop them on every exit: a start failure used to return before the unset and leave the
    # pin exported for every later script.
    pinned=()
    release_pins() {
        ((${#pinned[@]} == 0)) || unset "${pinned[@]}"
        pinned=()
    }
    while read -r pin; do
        [[ -n $pin ]] || continue
        export "$pin"
        pinned+=("${pin%%=*}")
    done < <(sed -n 's/^# env: //p' "$file")

    if ! (start_daemon) >"$log" 2>&1; then
        echo "FAIL $name daemon did not answer within ${DAEMON_START_TIMEOUT_S}s"
        if daemon_alive; then
            echo "    daemon pid $(daemon_pid) still running"
        else
            echo "    daemon pid $(daemon_pid) not running"
        fi
        echo "    script.log:"
        sed 's/^/    | /' "$log" 2>/dev/null || true
        echo "    daemon.log:"
        sed 's/^/    | /' <<<"$(tail -20 "$XDG_STATE_HOME/brr/daemon.log" 2>/dev/null)"
        FAILED=1
        teardown
        release_pins
        return
    fi

    timeout "$SCRIPT_TIMEOUT_S" bash "$file" >>"$log" 2>&1
    rc=$?
    reason=$(grep -m1 '^FAIL: ' "$log" || grep -v '^[[:space:]]*$' "$log" | tail -1)

    local daemon_log=$XDG_STATE_HOME/brr/daemon.log
    local daemon_tail script_tail
    daemon_tail=$(tail -15 "$daemon_log" 2>/dev/null)
    script_tail=$(tail -30 "$log")
    teardown
    release_pins

    if ((rc == 0)) && [[ -n $TEARDOWN_NOTE ]]; then
        rc=1
        reason=$TEARDOWN_NOTE
        echo "$reason" >>"$log"
    fi
    case $rc in
    0) echo "PASS $name ${reason#ok: }" ;;
    77) echo "SKIP $name $reason" ;;
    *)
        echo "FAIL $name ${reason:-exit $rc}"
        sed 's/^/    | /' <<<"$script_tail"
        echo "    daemon.log tail:"
        sed 's/^/    | /' <<<"$daemon_tail"
        FAILED=1
        ;;
    esac
}

NAMES=("$@")
TEARDOWN_NOTE=
matched=0
for file in "$HERE"/[0-9][0-9]-*.sh; do
    selected "$file" || continue
    matched=1
    run_one "$file"
done

if ((matched == 0)); then
    echo "FAIL run no script matches: ${NAMES[*]}"
    exit 1
fi
exit "$FAILED"
