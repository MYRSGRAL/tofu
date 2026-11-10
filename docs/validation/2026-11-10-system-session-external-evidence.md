# PR49: диагностика external Evidence

База этого среза: `85e97f54a81dd7fe8bb04a6fd78aac4d0061e9fc`, ветка
`feat/system-session-runtime`. Head и tree диагностической сборки сохранены
в `local/validation/external-diagnostic/provenance.json` после commit.

## Установлено и не установлено

Прочитан actual report `local/validation/system-session-vm-gate/state-acceptance-85e97f54/report.md`
и последовательность case11 в `matrix-serial.log`. На **85e97f54** diagnostic
прошёл; matrix завершилась ошибкой: 10 PASS, 11-й FAIL, 12-й NOT RUN.
Все 10 завершённых traffic groups прошли TCP/UDP IPv4/IPv6. Case11 — sing-box,
kill switch=true, ending=restart — отказал ещё в Apply: external verification
Evidence → Unavailable; restart/traffic/State не достигнуты. Error cleanup
успешен, осталась одна restrictive public запись согласно kill switch.

**Records / projection / equivalence: конкретная причина pending.** Старые
логи объединяют эти три проверки; снимков baseline/projected/current в них нет.
Ни ошибка нормализации, ни настоящее изменение RA/DHCP/expiry после примерно
двух минут не доказаны. Namespace RED для продуктового исправления не заявлен:
подтверждённого дефекта пока нет, этот commit только диагностический.
Historical Xray Unavailable на 02dfec9 и SSH hang остаются открытыми.
Sampler empty-cgroups-before-journal-decrease относится к старому actual run,
а не является новой проверкой этого head или доказательством всех syscall.
P09/P12/P13 остаются partial; полной приёмки system_tun нет.

## Границы изменений

`verify_external_captured` сохраняет исходные predicates и ошибки. Под cfg(test)
выделены стадии records, projection, equivalence. Для records дополнительно
показывается соответствие ожидаемым session descriptors и ownership; для
projection — исходный current; для equivalence — baseline/projected и raw
current. Классовый summary использует существующие same_link/address/route,
с исходными capture windows и expiry fencing. Повторное вычисление в summary
помечено как recomputed: оно не заменяет исходное решение.

Дамп возможен только после существующих root/marker/QEMU/systemd/cgroup/flock
проверок guest_guard и при `YONIK_SESSION_DIAGNOSTICS=1`. Бюджет — 32 КиБ на
scope, не более 16 элементов каждого класса/изменённых атрибутов/offsets.
Выводятся числовые синтетические IP/route metadata, capture windows относительно
baseline, USER_HZ, whitelisted cacheinfo/expires/TUN/IPv6 значения; для opaque
атрибутов — kind, длина, изменённые offsets без payload. Имена, alias,
credentials/config/hostname не выводятся. Environment flag сам по себе
не разрешает topology dump на host. Бюджет и отсутствие opaque strings,
отказ malformed/duplicate nested NLA проверяются unit-тестами.

## Следующий exact case — назначает root

После штатной подготовки disposable guest/peer/registry/pinned engines,
существующего guestguard и sampler, один запуск готового libtest:

```sh
YONIK_SYSTEMD_VM_GATE=1 YONIK_SESSION_DIAGNOSTICS=1 \
  /opt/yonik-gate/yonik-linux-tests --ignored --exact \
  helper::service::tests::installed_session::diagnose_sing_box_protected_apply \
  --nocapture --test-threads=1
```

Case требует доступный synthetic endpoint до effects, делает один Prepare/Apply
sing-box с kill switch=true, затем один Stop. При ошибке сохраняет restrictive
policy для evidence; Unlock при ошибке не выполняется. Успех требует настоящие
состояния tunnel_starting/recovery_required, затем авторизованный Unlock и
assert_clean. Никаких Apply/Stop retries и обхода fencing.

Успех одиночного case не исключает зависимость от предыдущих 10 cases/возраста
RA/DHCP. Root может отдельно назначить один existing matrix run с тем же
`YONIK_SESSION_DIAGNOSTICS=1`: guestguard активирует ту же диагностику, сохраняя
исходный loop. Здесь VM, matrix и host network effects не запускались.

## Локальная проверка и артефакты

Все Cargo-команды: Rust 1.85.1, CARGO_BUILD_JOBS=2,
CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/yonik-external-diagnostic/target,
под flock -s самого task-root. Проверки fmt, workspace tests, clippy all-targets
с -D warnings и diff-check; окончательные результаты и SHA бинарника — в
`local/validation/external-diagnostic`. Готовый exact libtest сохранён вне TTL
для root без новой сборки. Generated logs/бинарник не включаются в git.

Namespace gates повторно не запускались: новый срез добавляет только cfg(test)
диагностику и guarded ignored case; production comparison/cleanup/admission
не меняются. Это не подтверждение нового actual run и не исправление Evidence.

Фактические результаты Rust 1.85.1:

- `cargo +1.85.1 test --workspace --locked -- --test-threads=1`: 577 passed, 0 failed, 28 ignored.
- Новые privacy/budget/NLA unit-тесты: 2 passed.
- `cargo +1.85.1 clippy --workspace --all-targets --locked -- -D warnings`: exit0.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check`: exit0.
- `libtest --list --exact` подтвердил наличие нового ignored case, без его запуска на host.
