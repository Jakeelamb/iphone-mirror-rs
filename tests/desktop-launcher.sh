#!/bin/sh
set -eu
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT HUP INT TERM
cp "$(dirname "$0")/../scripts/launch-desktop.sh" "$root/launch"
cat >"$root/iphone-mirror-rs" <<'MOCK'
#!/bin/sh
printf 'backtrace=%s\n' "$RUST_BACKTRACE" >&2
exit "$1"
MOCK
printf '#!/bin/sh\nexit 0\n' >"$root/notify-send"
chmod +x "$root/iphone-mirror-rs" "$root/notify-send"
export XDG_STATE_HOME="$root/state" PATH="$root:$PATH" RUST_BACKTRACE=1
logs="$XDG_STATE_HOME/iphone-mirror-rs"
for status in 0 7 101; do
    result=0
    sh "$root/launch" "$status" || result=$?
    test "$result" -eq "$status"
done
test "$(find "$logs" -name 'launch-*.log' | wc -l)" -eq 3
for log in "$logs"/launch-*.log; do
    grep -q 'backtrace=1' "$log"
    grep -q "exit_status=$(cat "$log.exit")" "$log"
    test "$(stat -c %a "$log")" = 600
    test "$(stat -c %a "$log.exit")" = 600
done
printf 'still running\n' >"$logs/launch-active.log"
count=0
while [ "$count" -lt 22 ]; do
    sh "$root/launch" 0
    count=$((count + 1))
done
test "$(find "$logs" -name 'launch-*.log.exit' | wc -l)" -eq 20
test "$(find "$logs" -name 'launch-*.log' | wc -l)" -eq 21
grep -q 'still running' "$logs/launch-active.log"
printf 'PASS: logging, backtrace, exit status, permissions, retention and active-log preservation\n'
