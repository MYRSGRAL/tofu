#!/usr/bin/bash
set -euo pipefail
export YONIK_NFT_TEST_HOST_NETNS="$(stat -Lc '%d:%i' /proc/self/ns/net)"
exec unshare --user --map-root-user --net --mount --pid --fork --mount-proc sh -ec '
mount --make-rprivate /
mount -t tmpfs tmpfs /run
mount -t tmpfs tmpfs /tmp
mount -t sysfs sysfs /sys
exec "$1" "$2" --ignored --nocapture
' sh "$1" "${2:-private_bindings}"
