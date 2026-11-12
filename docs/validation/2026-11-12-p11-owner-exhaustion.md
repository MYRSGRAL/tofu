# P11: actual initialize owner Exhausted

База: 92f51a33b3f223da4c84f2b728e0f2e37fb4fba6, parent PR #132.
Отдельный example p11schedulerOwnerE2E использует ephemeral key и приватную БД.
Feature test-support открывает отдельный constructor HostConfig с private seed.
Локальный AtomicU64 передаётся в тот же production fetch_update/checked_add.
Обычный new/bounded использует прежний static NEXT_OWNER. Глобальный counter,
pins, capabilities, ABI, catalog/stages и зависимости не меняются.

MAX немедленно даёт настоящий ScheduleFailure/SchedulerInitializeExhausted.
Production actor выставляет startup в error/source, typed scheduler_initialize
contexts с generation=null и закрывает service. Production Bridge копирует strict
V2 в native clipboard; независимый Python validator принимает RU/EN exports.
Текст извлечён из production Main.qml. Initialize без generation поддерживается:
fail-closed gap в этом case отсутствует, generation не фабрикуется.

Positive seed=7 проходит ready → shutdown → closed без error contexts.
Успешный initial scheduler не публикует scheduler_generation; проверка точного
identity MAX−1 и последующего Exhausted выполняется Rust-тестом allocator.
Другой Rust-тест проверяет сохранение MAX после отказа.

Observer вызывается синхронно при snapshotChanged: production C++ завершает
приложение сразу после closed, поэтому timer-only observer не успевал проверить
Exhausted. Предварительные неуспешные observer-прогоны сохранены в evidence.

Запуск каждого режима с новым TASK:
```sh
unshare -Urnpf --mount-proc dbus-run-session -- env  QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software PYTHONPATH=tools  python3 tools/tests/check_p11schedulerOwnerE2E.py BINARY TASK exhausted
# либо positive
```

Evidence и сохранённый binary:
 /home/myrs/projects/yonik/local/validation/p11-owner-exhaustion/

Backend library MSRV 1.85.1 test-support: 91 PASS, 1 ignored.
GUI all-target tests не повторялись: известная базовая ошибка линковки FFI Bridge
из #132 остаётся unsupported target, PASS этому target не присваивается.
Android, visible compositor и физические G-UI/G-POWER отложены.
Полная Corrupt/Full/interval campaign не запускалась.

Финальные проверки: GUI all-targets clippy MSRV 1.85.1 gui,test-support
offline/locked -D warnings PASS (generated C++ Qt warnings сохранены в логе).
cargo fmt --all --check PASS. Pinned Ruff 0.17.4 check и format --check PASS.
Финальная GUI build PASS. Source/binary SHA256 — evidence/source-binary.json.
