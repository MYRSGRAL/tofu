#!/usr/bin/bash
set -euo pipefail
: "${1:?Укажите абсолютный путь тестового бинарника}"
: "${2:?Укажите фильтр одного namespace-теста}"
if [[ "$("$1" "$2" --ignored --list | tail -n 1)" != "1 test, 0 benchmarks" ]]; then
    echo "Один запуск требует ровно один тест в отдельном namespace" >&2
    exit 2
fi
export YONIK_NFT_TEST_HOST_NETNS="$(stat -Lc '%d:%i' /proc/self/ns/net)"
exec unshare --user --map-root-user --net --mount --pid --fork --mount-proc sh -ec '
mount --make-rprivate /
mount -t tmpfs tmpfs /run
mount -t tmpfs tmpfs /tmp
mount -t sysfs sysfs /sys
export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/yonik-private-bus
dbus-daemon --session --nofork --address="$DBUS_SYSTEM_BUS_ADDRESS" >/dev/null 2>&1 &
count=0
while [ ! -S /run/yonik-private-bus ]; do
    count=$((count + 1))
    [ "$count" -lt 100 ] || exit 1
    sleep 0.01
done
exec "$1" "$2" --ignored --nocapture
' sh "$1" "$2"
