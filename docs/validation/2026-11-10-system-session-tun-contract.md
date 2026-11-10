# Детерминированный TUN до worker attach — 10 ноября 2026

База `a2cce8ef583fd67d42c795b6cfb935b040666050`, PR #49.
Actual evidence прочитано из
`local/validation/system-session-vm-gate/diagnostic-a2cce8/report.md`
и `stable-diff.json`. Один actual Xray case локализовал Prepared(Stale) в
same_link_contract: VNET_HDR 0→1, addrgen mode 0→3 и IF_RS_SENT.
Worker start/pidfd/observe/status/topology/own-link прошли. Исторический
Unavailable на 02dfec9 этим результатом не воспроизведён и не объяснён;
он остаётся открытым. Stubborn gate повторно не запускался.

## Контракт framing

VNET_HDR задаёт virtio header перед каждым пакетом, это не косметическая metadata.
Универсальное включение VNET_HDR для обоих ядер было бы ошибкой.

- Pinned [Xray v26.10.31 tun_linux.go](https://github.com/XTLS/Xray-core/blob/v26.10.31/proxy/tun/tun_linux.go)
  открывает IFF_TUN|IFF_NO_PI|IFF_VNET_HDR. fdTun использует vnetSize=10:
  [tun_fd.go](https://github.com/XTLS/Xray-core/blob/v26.10.31/proxy/tun/tun_fd.go).
  Upstream имеет fallback без VNET_HDR при ошибке; текущий helper ожидает
  проверенный VNET_HDR путь. Fallback, меняющий framing, даст отказ fencing.
- Pinned sing-box v1.14.4 использует sing-tun `040db71fcd33`.
  [inbound.go](https://github.com/SagerNet/sing-box/blob/v1.14.4/protocol/tun/inbound.go)
  включает GSO только для явно выбранного stack=gvisor и малого MTU.
  Текущий compile_system не задаёт этот stack, поэтому GSO=false.
  [sing-tun open](https://github.com/SagerNet/sing-tun/blob/040db71fcd33/tun_linux.go)
  добавляет IFF_VNET_HDR только при options.GSO. Здесь ожидается plain IP, VNET_HDR=0.
  Existing external_configuration patch сохраняет эту FD-семантику, отключая
  конкурирующее управление адресами/routes/DNS. В текущем runtime worker получает sealed binary/config memfd; TUN queue
  открывает само ядро по helper-selected имени. Произвольный TUN FD, path,
  argv/env клиент не передаёт. Xray optional env-FD и sing-box FileDescriptor
  здесь не включены; проверяется именно open-by-name путь.

## Изменение

Private SessionNetworkPlan выбирает VNET_HDR по typed engine до компиляции и
создания immutable artifact. TUN создаётся сразу с нужным ip tuntap vnet_hdr
флагом для Xray; sing-box и legacy reserve остаются без него.
Проекция own topology принимает точное ожидаемое значение из плана,
а не глобальную allowlist 0/1. same_link_contract продолжает сравнивать VNET_HDR.

Owned configure_link через RTM_SETLINK заранее задаёт AF_INET6 addrgen mode NONE
и проверяет это actual dump после ACK. Только затем baseline сохраняется и
запускается worker. Нативная IPv6 link-local autoconfiguration/RS не нужна:
helper сам назначает оба адреса. Alias/namespace/journal ownership guards сохранены.
Baseline после worker start не обновляется; AF_SPEC/CONF/TOKEN/unknown attrs
и поколения не маскируются. Durable journal, cgroup-before-cleanup и readiness
не менялись. Дополнительные зависимости, backend/QML и host settings не менялись.

## RED → GREEN и отрицательные проверки

Первый namespace regression на исходном production коде открыл настоящую очередь
с IFF_VNET_HDR (0x5001): same_link_contract RED, safe runner exit1/lib-test exit101.
В red.log есть actual VNET_HDR и AF_SPEC diff; fixture control отсутствовал.

После исправления тот же namespace gate проходит для двух engine-mode bindings:
Xray flags 0x5001 и sing-box flags 0x1001. Это настоящие TUN FD attach,
а не запуск настоящих engines. Для VNET_HDR дополнительно проверен
TUNGETVNETHDRSZ=10. Gate применяет owned addresses/routes/RPDB, затем проверяет
полную проекцию и стабильный link contract после открытия очереди.
Подмена alias/index и unknown attr отклоняются. Отдельный unit test проверяет
отказ при VNET_HDR 0→1 и 1→0; прежние address/route renewal/expiry/replacement
и generation tests остаются в workspace suite.

Новый exact lib-test и provenance сохраняются вне TTL в
`local/validation/tun-contract-fix/yonik-linux-tests` и `binary-provenance.json`.
Следующий actual case назначает root; можно использовать тот же
`helper::service::tests::installed_session::diagnose_first_xray_apply`
с YONIK_SYSTEMD_VM_GATE=1 и YONIK_SESSION_DIAGNOSTICS=1 только внутри disposable VM.
Никаких fixture addrgen controls для нового actual case не требуется.

Полная actual engine TCP/UDP IPv4/IPv6 матрица и P09/P12/P13 не приняты;
`system_tun:false`. Новый fix не переносит old stubborn/traffic PASS на свой head.
Own target: `/var/tmp/codex-myrs/transient/yonik-tun-contract-fix/target`, jobs=2,
root flock. Логи и lib-test копируются вне TTL до cargo clean.

Проверки (Rust 1.85.1):

- `cargo +1.85.1 test --workspace --locked -- --test-threads=1`: 565 passed, 0 failed, 27 ignored.
- `cargo +1.85.1 clippy --workspace --all-targets --locked -- -D warnings`: exit 0.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check`: exit 0.
- `tools/managed_network_smoke.py <lib-test> --case session-attach`: RED до fix;
  GREEN после fix, 1 passed, без addrgen fixture control.
- Тот же session-attach с `YONIK_TUN_ATTACH_ENGINE=sing-box`: GREEN, 1 passed.
- Прежние namespace cases session-network, tun-projection, tun-config, service:
  каждый exit 0, один actual namespace test.

Полные логи: red.log, xray-attach.log, sing-box-attach.log, workspace-tests.log,
clippy.log, gate-*.log, xray-source.log. Оба GREEN являются synthetic queue attach,
не actual pinned core traffic. Root назначает следующий actual case и review.
