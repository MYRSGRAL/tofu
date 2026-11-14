#!/usr/bin/bash
# Вызывается только внутри собственного user/PID/mount/net namespace.
set -euo pipefail
work=$1
binary=$2
project=$3
export A08_CASE=${4:-limited}
mount --make-rprivate /
mount --bind "$work" /mnt
mount -t tmpfs tmpfs /tmp
mount --bind /mnt/empty-apps /usr/share/applications
mkdir -p /mnt/localshare/applications
mount --bind /mnt/localshare /usr/local/share
export HOME=/mnt/home XDG_DATA_HOME=/mnt/xdg XDG_CONFIG_HOME=/mnt/config XDG_RUNTIME_DIR=/mnt/runtime
export QT_FORCE_STDERR_LOGGING=1
export QT_QPA_PLATFORM=xcb QT_QUICK_BACKEND=software LIBGL_ALWAYS_SOFTWARE=1 DISPLAY=:91 A08_ROOT=/mnt/host
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_RUNTIME_DIR" /mnt/host
chmod 700 "$XDG_RUNTIME_DIR" /mnt/host
/mnt/Xvfb :91 -screen 0 1440x1200x24 -nolisten tcp > /mnt/xvfb.log 2>&1 &
xvfb=$!
/mnt/helper &
helper=$!
trap 'kill "$helper" "$xvfb" 2>/dev/null || true; wait "$helper" "$xvfb" 2>/dev/null || true' EXIT
for iteration in {1..100}; do
    [[ -S /tmp/.X11-unix/X91 ]] && break
    sleep 0.02
done
[[ -S /tmp/.X11-unix/X91 ]]
[[ ! -e /mnt/marker-control ]] || unlink /mnt/marker-control
/mnt/helper /mnt/marker-control
[[ -f /mnt/marker-control ]]
python3 "$project/tools/a08_proc_trace.py" /mnt/canary.jsonl /mnt/helper canary
export LD_PRELOAD=/mnt/observer.so
timeout --signal=TERM --kill-after=3 35 python3 "$project/tools/a08_proc_trace.py" /mnt/proc.jsonl "$binary" /mnt/host
[[ ! -e /mnt/EXEC-MUST-NOT-EXIST ]]
