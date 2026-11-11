# P11: реальный отказ DesktopHost

База: `6cf65a4cf9760a7df68b8eee10f012770b38604d`. Изменения ограничены
errorText и отдельными fixture/gate; Scheduler, Routing, settings, SystemSession,
maintenance и presets не меняются.

`check_diagnostic_desktop_failure.py` запускает production GUI без smoke-test
и без ephemeral key. Собственный каталог данных с mode=0755 проходит CLI mkdir,
но отклоняется `backend::private_directory` до доступа к Secret Service и БД.
Начальный QML загружается interposer, однако `DesktopBridge.initialize()` и
snapshot создаёт настоящий DesktopHost. errorText извлекается из текущего
Main.qml без самостоятельного словаря. RU/EN проверяются через TextEdit,
экспорт — явным `copy_diagnostics()` и чтением настоящего Qt clipboard.
Negative canary подставляется только после этих проверок: это отдельный
контролируемый отказ экспортёра, а не источник production storage failure.
Инъекция cleanup этим сценарием не проверяется.

| Producer | Текущий закрытый путь | Покрытие |
| --- | --- | --- |
| startup | startup, listen_port_busy | manifest + factory gate |
| storage | storage, keyring_locked/missing, preferences_write | gate; actual mode rejection storage |
| catalog | catalog | явная RU/EN пара errorText + gate |
| System | system_unsupported/denied/owner_lost/timeout/unavailable, cancelled, busy | gate, без реального helper RPC |
| cleanup | cleanup_failed | gate; прежний controlled snapshot не является actual cleanup |
| operation ACK | routing_ack.error: cancelled/id/routing_stale/routing_storage и editor validation | gate, существующий exporter allowlist; добавлены ACTION/action и FORMAT/format |
| subscription / engine / probe | существующие литеральные DesktopError и error assignments | factory gate, без live network |

Manifest catalog_version=2 содержит 76 ключей; schema_version=1 сохранён.
Gate проверяет текущие литеральные фабрики backend и editor, обе match-фабрики
map_subscription/system_error, assignment errors и соответствие errorText
закрытому manifest. Это ограниченный source gate, не Rust AST-анализ;
динамическая новая фабрика потребует расширения gate.

## Остаток контракта для root

Текущий snapshot несёт строку и теряет место возникновения. `storage` не позволяет
отличить directory permissions от DB open; `system_timeout` не различает deadline
и timeout. Экспорт сохраняет schema1: stage=operation, retryable=false, params={}.
Этот stage не доказывает фактическую фазу. Reason отсутствует. Полный P11 не закрыт.

Узкое предложение до shared ABI edits: optional structured sibling для каждого
error slot/ACK с закрытыми enum stage/reason; строковые error/copyDiagnostics
сохраняются. Producer создаёт metadata в точке отказа, exporter проверяет
соответствие key/code и enum, а не угадывает фазу по message_key. Первые значения:
stage=storage_open, reason=data_directory_rejected (без различения errno,
пока producer его не сохраняет). Ни URL, ни argv, ни native cause в params.
Введение stage/reason требует согласованного обновления strict validator;
в этом изменении validator и ABI не изменены.

Категории спецификации DNS_POLICY_AMBIGUOUS/UNRESOLVED, конфликт маршрутов,
endpoint и unsupported capability требуют отдельных producer proofs.
Они не объявляются покрытыми общим unknown; отсутствующий UI key теперь явно
показывает отсутствие категории в каталоге, без вывода входного ключа.

## Actual evidence

Production GUI Rust 1.85.1, `--offline --locked --features gui`, без test-support.
Binary SHA256: `96447a320901ddb15ac70b0a64f757138fcc06c156271ab56a874798fbbdb68f`.
Actual GUI trace подтвердил обе локали, два одинаковых безопасных экспорта
`STORAGE/storage`, params={}, retryable=false и отрицательный clipboard canary.
Сценарий выполнен внутри `unshare -Urnpf --mount-proc dbus-run-session` с
`QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software`.

Evidence и binary вне TTL:
`/home/myrs/projects/yonik/local/validation/p11-actual-f1fcd/`.
Там source-manifest.json, producer-keys.json (35 извлечённых ключей),
desktop-failure.qml, actual.log (финальный), actual-early.log, build-final.log, Qt/Python/translations logs.
Начальные два отказа fixture связаны с длиной Unix socket path, а не DesktopHost;
текущий runner использует короткий собственный TemporaryDirectory для runtime.
Qt6 qmltestrunner: один поведенческий stub-сценарий PASS плюс init/cleanup;
этот результат отделён от actual DesktopHost trace.

Повторение (TASK — свой transient каталог под BSD flock):

```sh
unshare -Urnpf --mount-proc dbus-run-session -- env \
  QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software \
  python3 tools/tests/check_diagnostic_desktop_failure.py BINARY TASK
python3 -m unittest discover -s tools/tests -p test_p11_factory_coverage.py -v
```

Финальные проверки: Rust diagnostics 8 PASS; clippy Rust 1.85.1
`--offline --locked -p yonik-desktop --features gui --all-targets -- -D warnings`
PASS (без --no-deps); Python export 10 PASS; factory gate PASS; Ruff и cargo fmt
PASS. Qt headers GCC16 дают compiler warnings, Rust lint suppressions не добавлены.

Перед очисткой /proc cwd/exe/fd: 0 ссылок на собственный transient root.
Cargo clean завершился успешно; task-каталог удалён, binary и evidence сохранены.
