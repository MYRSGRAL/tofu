#!/usr/bin/bash
# Запуск только указанного N4 test в собственных user/net/mount namespaces.
set -euo pipefail
if (( $# != 3 )); then
    echo 'Нужны Rust test ELF, Xray test ELF, sing-box test ELF' >&2
    exit 2
fi
export YONIK_NFT_TEST_HOST_NETNS
YONIK_NFT_TEST_HOST_NETNS=$(stat -Lc '%d:%i' /proc/self/ns/net)
export YONIK_N4_XRAY="$2" YONIK_N4_SING_BOX="$3"
exec unshare --user --map-root-user --net --mount --fork /usr/bin/bash -euc '
    mount --make-rprivate /
    mount -t tmpfs -o mode=0700 tmpfs /tmp
    ip link set lo up
    exec "$1" --exact managed_network::tun::custody::tests::real_journal_actual_consumer --ignored --nocapture --test-threads=1
' n4 "$1"
