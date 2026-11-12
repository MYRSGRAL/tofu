# Actual maintenance generation: незакрытый blocker

База PR136: 45dc06d23fb4e85392b9fe3908ff0b2e7790a47a. Только Linux.
Это диагностический результат, а не доказательство stale fence.

Observer tools/tests/check_maintenance_generation_ui.py загружает неизменённый
qrc Main, проверяет его SHA256, читает actual backend snapshots и native clipboard.
Настоящий interval validator получает ZERO из retained example PR136; A=1,
оба scheduler_validation contexts имеют generation="1". Штатная кнопка copy
успешно экспортирует strict V2. Затем Settings → MaintenanceView.submit отправляет
настоящий backup с новым destination (пути формы задаёт observer, FileDialog не
автоматизируется). Snapshot/context/generation не присваиваются.

Два запуска дали request_id=1 draining → failed/storage, ready=false,
maintenance_available=false, generation остаётся 1. Второй запуск корректно
вызывает shutdown и требует closed snapshot и terminal Qt exit. Backup не
опубликован; healthy verified backup, restore, replacement B и отказ stale copy
не доказаны. Первый запуск сохранил deadline failure; повтор не менял backend,
только обработку failed ACK для штатного shutdown. Успех процесса observer
означает сохранённое наблюдение, а не успешный backup.

Evidence вне TTL:
/home/myrs/projects/yonik/local/validation/p11-maintenance-generation/terminal-ru/
(observations.json, qt.log, observer.cpp/.so, driver.qml, encrypted fixture).
Первый failed прогон: соседний actual-ru/. Синтетический ephemeral key в памяти,
SecretService/TUN/physical proof не заявляются.
Retained binary:
/home/myrs/projects/yonik/local/validation/p11-interval-132/p11schedulerIntervalE2E
SHA256 e32c4cd2554f3e0f4b68bfd6379f5082e5ae62a08199835477c45853fc5c26ce.
Rust inputs в этой ветке не менялись; Cargo/build/новый transient root не создавались.
MSRV1.85.1 all-targets clippy -D warnings и fmt для этого binary сохранены в
parent evidence p11-interval-132/{clippy,fmt}.log; повторные проверки Rust не выполнялись.
Pinned Ruff 0.17.4 check и format --check observer: PASS.

## Предложение root до дальнейших Rust изменений

1. Разрешить отдельный закрытый example mode без нового HostConfig/enum/ABI:
одноразовый ZERO на startup, затем поддерживаемый interval после настоящего
maintenance drain. Требуется приватный test-support timing barrier в supervisor:
после accept_maintenance_owner и emit complete ACK, до следующего
runtime.set_interval. Управление — только example-owned synchronisation, без
Bridge/env/preferences входа; callback не присваивает diagnostics.
2. Сначала локализовать failed/storage при настоящем backup с existing fixture;
не обходить его поддельным ACK или replacement. Нужен matching successful
backup и проверенная копия перед restore.
3. Пока barrier удерживает итерацию B, Main copy должен отказать на сохранённом
A и оставить clipboard без изменения. После release и supported interval
проверить отсутствие экспорта A. Без barrier ZERO повторно применяется в начале
каждой итерации backend.rs:3502–3505, поэтому fault B может сразу заменить A;
это само по себе старый fence не доказывает.

Новый hook требует согласования root. Pins/capabilities/ABI/dependencies не менялись.
RU/EN два успешных lifecycle cases, Android и физические gates остаются открытыми.
