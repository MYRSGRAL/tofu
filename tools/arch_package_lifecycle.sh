#!/bin/bash
# Только disposable guest: контроль установки и сохранности чужих данных.
set -euo pipefail
cd /home/builder/input
package=(packaging/arch/*.pkg.tar.zst)
inactive() {
    local state status=0
    state=$(systemctl is-active yonik-helper.service) || status=$?
    test "$status" = 3 && test "$state" = inactive
}
case "$1" in
    installed)
        inactive
        getent passwd yonik-worker
        systemd-analyze verify /usr/lib/systemd/system/yonik-helper.service
        test -f /usr/share/dbus-1/system.d/cloud.highpriority.yonik.Network1.conf
        test -n "$(find /usr/share/polkit-1/actions -name '*yonik*' -print -quit)"
        ;;
    cycles)
        install -d -o builder -g builder /home/builder/.local/share/yonik
        printf 'own-db-sentinel\n' > /home/builder/.local/share/yonik/state.db
        printf 'own-key-sentinel\n' > /home/builder/.local/share/yonik/keyfile
        chmod 600 /home/builder/.local/share/yonik/keyfile
        sha256sum /home/builder/.local/share/yonik/* > /home/builder/sentinel.sha256
        nft add table inet yonik_foreign_fixture
        ip link add yonik-foreign type dummy
        trap 'ip link del yonik-foreign; nft delete table inet yonik_foreign_fixture' EXIT
        # Повторная установка того же build проверяет package transaction;
        # версионный rollback требует отдельного предыдущего real package.
        pacman -U --noconfirm "${package[0]}"
        inactive
        pacman -Qkk yonik
        pacman -R --noconfirm yonik
        test ! -e /usr/bin/yonik-desktop
        sha256sum -c /home/builder/sentinel.sha256
        nft list table inet yonik_foreign_fixture
        ip link show yonik-foreign
        ;;
    *) exit 2 ;;
esac
