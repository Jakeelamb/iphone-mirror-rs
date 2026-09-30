#!/bin/sh
# Keep desktop-launch errors available even when stderr is /dev/null.
umask 077
state_dir=${XDG_STATE_HOME:-"$HOME/.local/state"}/iphone-mirror-rs
mkdir -p "$state_dir" || exit 1
log=$(mktemp "$state_dir/launch-XXXXXX.log") || exit 1
binary=$(dirname "$0")/iphone-mirror-rs
printf 'iOS Mirror launch: %s UTC\n' "$(date -u '+%Y-%m-%d %H:%M:%S')" >"$log"
printf 'launcher_pid=%s binary=%s\n' "$$" "$binary" >>"$log"
RUST_BACKTRACE=${RUST_BACKTRACE:-1} "$binary" "$@" >>"$log" 2>&1
result=$?
printf '\nexit_status=%s finished=%s UTC\n' "$result" "$(date -u '+%Y-%m-%d %H:%M:%S')" >>"$log"
printf '%s\n' "$result" >"$log.exit"
# Retain twenty completed runs. Active logs have no receipt and are never removed.
ls -1t "$state_dir"/launch-*.log.exit 2>/dev/null | tail -n +21 |
    while IFS= read -r receipt; do
        rm -f -- "$receipt" "${receipt%.exit}"
    done
if [ "$result" -ne 0 ]; then
    if command -v notify-send >/dev/null 2>&1; then
        notify-send -u critical 'iOS Mirror stopped' "Launch or session failed. Details: $log" || :
    fi
fi
exit "$result"
