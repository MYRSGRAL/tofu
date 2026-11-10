# Системный runtime сессии — 10 ноября 2026

База: `e33fa1908dfb3deb5434506341674de0b3a827b4`.
Ветка: `feat/system-session-runtime`, существующий основной checkout.
Переданная незакоммиченная реализация сохранена; этот PR включает весь её diff.
Head реализации: `5a86cbd9319101f259c4a643b206b6e96323bdca`.
Итоговый head PR дополнительно включает отдельный docs commit с этим отчётом.

## Границы

Типизированный `system_profile` Prepare/Apply/Stop использует существующий
SystemProtectionHost как единого владельца backend/journal. Отдельный модуль
session_host содержит immutable artifact (sealed payload, baseline, маршрут,
поколение, UID, plan и бюджет), worker supervisor и worker-first recovery.
Journal v3 хранит kill-switch решение и строго читает исторические v1/v2.
Политика public protection и session resources имеют один durable journal.

Изменены только plan.rs приложения, helper/*, helper_main.rs, managed_network/*,
network.rs, network/* и tests/network_journal.rs; добавлены session_host,
session_host_tests, installed_session_tests, session_state и equivalence.
Зависимости и Cargo.lock не изменены. GUI/client, subscriptions, packaging и
общий PLAN в этот PR не входят.

## Проверки

Все Cargo-команды выполнялись с `CARGO_BUILD_JOBS=2`,
`CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/yonik-system-session-final/target`
под `flock -s` самого task-root. Toolchain — 1.85.1.

- `cargo +1.85.1 test -p yonik-linux -p yonik-application`: 401 passed,
  0 failed, 24 ignored. Ignored не засчитываются как выполненные.
- `cargo +1.85.1 clippy --workspace --all-targets --locked -- -D warnings`: exit 0.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check`: exit 0.
- `python3 tools/managed_network_smoke.py <lib-test-binary> --case session-network`:
  exit 0, один actual namespace test.
- Тот же runner, `--case tun-projection`: exit 0, один actual namespace test.
- Тот же runner, `--case service`: exit 0, один actual namespace test.
  dbus-daemon сообщает недоступность inotify /root в sandbox; тест проходит.

Unit tests проверяют закрытый IPC, свежую авторизацию/peer/generation,
неподдерживаемые hostname/underlay до эффектов, identity/capability worker,
удержание backend при отмене caller, порядок worker→restrict→network→unlock,
ошибки cleanup, durable disposition и pidfd/background safety.
Переданная реализация существовала до этого запуска; новый TDD-цикл для неё
не заявляется. В этом запуске изменено форматирование и язык test diagnostics.

Evidence вне TTL: `local/validation/system-session-final/` (не коммитится):
`tests.log`, `clippy.log`, `workspace-tests.log`, `gate-session-network.log`,
`gate-tun-projection.log`, `gate-service.log`.

Обычный `cargo +1.85.1 test --workspace --locked` завершился exit 101:
`helper::tests::timeout_cancels_remote_check_and_ignores_late_allow` получил
Elapsed в next_call (5 секунд). В его авторизации установлен deadline 100 ms;
при параллельной нагрузке запрос может не дойти до ожидаемой фазы fixture.
Отдельный повтор этого теста прошёл (1 passed, 0 failed, 0.13 s).
Это наблюдение о чувствительности теста к нагрузке, не доказанная причина.
Продакшен-таймауты и существующий тест не изменены.
Исходный красный лог сохранён в `workspace-tests.log`, узкий повтор —
`timeout-recheck.log`; последовательный полный прогон — `workspace-serial.log`.

Последовательный полный `cargo +1.85.1 test --workspace --locked -- --test-threads=1`:
exit 0, 557 passed, 0 failed, 25 ignored.

## Непройденная внешняя граница

Installed session gate `real_installed_profile_rpc_tunnel_and_restart_cleanup`
не запускался: прежняя disposable VM штатно выключена и удалена, включая
image/overlay/SSH key. Проверено отсутствие image по пути прежнего runner
`/var/tmp/codex-myrs/transient/yonik-session-runtime/image/Arch-Linux-x86_64-cloudimg.qcow2`.
`/dev/kvm` существует; для нового прогона требуется заново подготовленная
проверенная одноразовая QEMU VM и синтетический VLESS peer по существующему
`local/validation/session-runtime/prepare_vm.py` и guest setup.
На host helper/units не устанавливались, сеть/keyring/подписки не менялись.

Последний сохранённый прошлый прогон в `local/validation/session-runtime/`
дошёл до Check и завершился Prepare/Stale; он не доказывает Apply.
Консервативное сравнение конечных address lifetimes может инвалидировать
artifact при DHCP/SLAAC. Ослабление generation/expiry fencing не выполнялось.

TERM→KILL для упрямого потомка и полная матрица installed Prepare/Apply/Stop,
worker exit, restart, cancel/error cleanup требуют реального VM-доказательства.
Предыдущие installed-worker gates также явно не моделировали упрямого потомка.
Namespace service gate проверяет public-protection, не весь system-profile путь.

`TunnelStarting` не означает транспортную готовность. `system_tun:false`
сохранён: actual TUN TCP/UDP IPv4/IPv6 обоих ядер через новый session host,
DNS/Fake-IP, live topology и client/GUI остаются отдельной приёмкой.
P09/P12/P13 полностью готовыми не объявляются. До merge нужен независимый
review combined diff base..head и решение root по непройденной VM-матрице.
