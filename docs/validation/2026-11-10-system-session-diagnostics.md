# Диагностика post-start Apply — 10 ноября 2026

База: `02dfec9aaf22e8cb9c6cc375873a1a8e77a8a25f`, PR #49,
ветка `feat/system-session-runtime`, основной checkout yonik.
Это диагностический срез, не принятое исправление Unavailable/Stale.

## Разделение failure branches

Прочитан actual VM отчёт
`local/validation/system-session-vm-gate/route-fix/report.md`.
Acceptance первый Xray Apply вернул Unavailable после запуска worker и cleanup.
Отдельный local-GDB run вернул Stale из same_link_contract. Связь двух ответов
не доказана. Новый VM rebuild и повтор stubborn gate здесь не выполнялись.
Stubborn TERM→KILL/Stop/BindsTo/cgroup-empty PASS относится именно к 02dfec9;
на новый диагностический commit это evidence не переносится.

## Локальная причинная проверка

Собственный user/net namespace, persistent TUN, link up, собственные адреса,
затем открытие одной очереди через /dev/net/tun (IFF_TUN|IFF_NO_PI).
Первый Python RTNETLINK probe сохранил raw attrs; затем Rust gate использовал
настоящие SessionNetworkPlan.configure и same_link_contract.
Перед любыми эффектами существующий fixture проверяет отличие от host netns.
Очередь закрыта и свои network records очищены до assertion.

`--case session-attach` воспроизводит намеренный RED: same_link_contract
отклоняет persistent TUN после attach. Это воспроизводимая известная регрессия,
она остаётся ignored и не объявляется исправленной.

В stable AF_SPEC отличаются:

- AF_INET6/IFLA_INET6_CONF word 47 (DEVCONF_ADDR_GEN_MODE): 0 → 3;
- AF_INET6/IFLA_INET6_ADDR_GEN_MODE (kind 8): 0 → 3;
- AF_INET6/IFLA_INET6_FLAGS: в одном снимке 0 → 0x10 (IF_RS_SENT).

Между raw снимками также меняются carrier, counters, IF_READY и timestamp;
существующая нормализация исключает эти поля. Неизвестные attrs сохранены.
Флаг IF_RS_SENT зависит от timing; его наличие не предполагается для всех runs.
Контроль `YONIK_TUN_DIAGNOSTIC_ADDRGEN_NONE=1` устанавливает addrgenmode none
только на свой TUN внутри namespace до baseline: тот же Rust gate проходит
GREEN. Это causal fixture control, production configure не изменён.

Первичный источник:
[Linux addrconf_dev_config](https://github.com/torvalds/linux/blob/v6.12/net/ipv6/addrconf.c)
для ARPHRD_NONE переводит EUI64 в RANDOM и вызывает addrconf_addr_gen.
[Linux tun_fill_info](https://github.com/torvalds/linux/blob/v6.12/drivers/net/tun.c)
передаёт TYPE/OWNER/GROUP/PI/VNET_HDR/PERSIST/MULTI_QUEUE; queue counts
публикуются только при IFF_MULTI_QUEUE. Этот single-queue probe не доказывает
отсутствие других изменений при attach настоящего engine.
Kernel source snippets сохранены в evidence. Fencing не ослаблен; auto-конфигурация
своего TUN — конкретный кандидат для следующего минимального исправления,
но исходный VM Unavailable требует отдельной локализации.

## Добавленная диагностика

Только lib-test при `YONIK_SESSION_DIAGNOSTICS=1`: результаты worker start,
pidfd watch, observe before/after, whitelist UID/capabilities/NoNewPrivs/State
при отказе process-status, read topology, наличие own link, external verification,
recover_public_policy и start_profile/error_cleanup. Стабильные и raw TUN
canonical fields и differing attrs печатаются с kind/length/value (cap 8192 байта
на attr). Worker payload, registry/config, credentials, hostname/endpoint,
argv/env и пользовательская topology не печатаются. Production binary не содержит
этих логов и не меняет решения проверок. Политика readiness не менялась.

## Один первый Xray case в disposable VM

Использовать immutable archive нового диагностического SHA и прежний безопасный
VM setup: pinned engines/registry/launcher/worker units, synthetic VLESS peer,
root-owned marker и private D-Bus/polkit fixture. Не включать fixture control
addrgenmode none: требуется диагностика неизменённого production пути.
Только внутри disposable VM:

```sh
YONIK_SYSTEMD_VM_GATE=1 YONIK_SESSION_DIAGNOSTICS=1 \
  /opt/yonik-gate/yonik-linux-tests --ignored --exact \
  helper::service::tests::installed_session::diagnose_first_xray_apply \
  --nocapture --test-threads=1
```

Путь lib-test бинарника согласовать с существующим VM staging. Этот новый test
исполняет только первый Xray Prepare/Apply, затем Stop; результат Apply сохраняет
для assertion после попытки Stop. При ошибке не продолжает traffic/матрицу.
Сохранить полный stdout/stderr и systemd journal. Если ветка worker_status,
observe или external_verification откажет раньше same_link_contract, считать
её отдельным failure. Этот диагностический тест не заменяет acceptance matrix.

## Evidence и проверки

Evidence вне TTL и git: `local/validation/system-session-diagnostics/`.
Исходный standalone probe и kernel snippets, attach-red.log, attach-control.log,
unit.log и runner-tests.log. lib-test бинарник и его provenance сохраняются отдельно
для одного диагностического case. Собственный target — только
`/var/tmp/codex-myrs/transient/yonik-tun-diagnosis/target`, jobs=2 и BSD flock
самого task-root. После переноса evidence выполняется cargo clean.

Root назначает узкий review и actual диагностический rerun. P09/P12/P13 остаются
partial, `system_tun:false`. Причина Unavailable пока не установлена.

Проверки текущего диагностического среза (Rust 1.85.1):

- `cargo +1.85.1 test --workspace --locked -- --test-threads=1`: 564 passed, 0 failed, 27 ignored.
- `cargo +1.85.1 clippy --workspace --all-targets --locked -- -D warnings`: exit 0.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check`: exit 0.
- `python3 -m unittest tools.tests.test_managed_network_smoke`: 10 passed.
- Namespace gates session-network, tun-projection, service: каждый exit 0.
- Новый ignored session-attach без control: ожидаемый RED, lib-test exit 101,
  safe runner exit 1, assertion same_link_contract после штатного cleanup.
- Тот же session-attach с fixture-only addrgenmode none: GREEN, 1 passed.

Последние две строки являются causal диагностикой и не засчитывают исправление
production Apply. Полные commands/evidence — attach-red.log и attach-control.log.
