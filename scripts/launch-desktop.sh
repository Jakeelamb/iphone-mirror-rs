#!/bin/sh
# Keep desktop-launch errors available even when stderr is /dev/null.
umask 077
state_dir=${XDG_STATE_HOME:-"$HOME/.local/state"}/iphone-mirror-rs
mkdir -p "$state_dir" || exit 1
log=$(mktemp "$state_dir/launch-XXXXXX.log") || exit 1
binary=$(dirname "$0")/iphone-mirror-rs
"$binary" "$@" >"$log" 2>&1
result=$?
if [ "$result" -ne 0 ]; then
    if command -v notify-send >/dev/null 2>&1; then
        notify-send -u critical 'iOS Mirror stopped' "Launch or session failed. Details: $log" || :
    fi
fi
exit "$result"
