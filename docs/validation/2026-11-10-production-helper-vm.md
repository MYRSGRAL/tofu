# Production helper: отдельный VM gate

База: `dd6962e7618d688660927bc58e2694940eba5598`, tree
`62dbd9b6bf71455a87b85dbdc8727517f167930b`.

Добавлены `installed_helper_session.rs` и `tools/helper_vm_fixture.py`.
Fixture принимает payload с SHA256-манифестом и устанавливает неизменённые
production helper/worker units, system-bus policy и polkit actions. Проверка
hash полного набора предшествует установке. Существующие файлы не заменяются.
Авторизация выполняется настоящим polkit: VM-local rule разрешает два Yonik
действия root и отказывает остальным UID. Это временный заменитель интерактивного
polkit agent только в VM, не подмена authorizer в helper.

Gate использует raw zbus существующего IPC v1, одну connection для Prepare/Apply,
проверяет public protection Apply/State/Stop/Unlock, настоящий `/proc/PID/exe`,
SIGKILL процесса helper, новый PID и RecoveryRequired после восстановления.
Отдельный negative case требует непривилегированный UID и именно ошибку
`authorization`; timeout или недоступный сервис не считаются успешным отказом.
Host opt-in выключен по умолчанию. Требуются QEMU, systemd PID 1, безопасный
root-owned marker `/etc/yonik-production-helper-vm` с содержимым
`yonik-production-helper-disposable-v1\n` (здесь `\n` означает перевод строки).
Private system-bus address запрещён.

## Запуск внутри подготовленной VM

Создать marker при cloud-init, до установки payload. VM должна быть собственной,
одноразовой, с 2 CPU / 2 GiB и единственным controller; host runner должен держать
BSD flock своего transient task root. Payload содержит exact source archive,
свежие helper/worker и `sha256.json` с hash каждого устанавливаемого файла.
Manifest обеспечивает целостность переноса, но сам по себе не доказывает
происхождение бинарника: build provenance должен отдельно связывать head/tree,
toolchain и SHA256. Старые binary reuse без такой связи запрещены.

```sh
YONIK_PRODUCTION_HELPER_VM=1 python3 helper_vm_fixture.py --payload /opt/yonik-payload
YONIK_PRODUCTION_HELPER_VM=1 /opt/installed-helper-session --ignored --exact production_helper_public_policy_survives_sigkill --nocapture
runuser -u nobody -- env YONIK_PRODUCTION_HELPER_VM=1 /opt/installed-helper-session --ignored --exact production_helper_denies_unprivileged_uid --nocapture
```

После проверки controller сохраняет journal, nft/rule/link snapshots и hashes
в постоянный evidence-каталог; выключает VM и удаляет только собственный overlay.
При ошибке gate намеренно не снимает restrictive policy; уничтожение собственной
VM — обязательный cleanup. На host скрипт ничего не устанавливает.

## Фактическое состояние проверки

VM **не запускалась**. Старый transient каталог
`/var/tmp/codex-myrs/transient/yonik-system-session-vm-gate` отсутствует;
поиск qcow2/cloudimg в доступных `/home/myrs` и `/var/tmp` не нашёл cached image.
Сохранённая provenance старого образа есть в
`/home/myrs/projects/yonik/local/validation/system-session-vm-gate/external-c758bd2/image.json`,
но самого файла нет. Host download не выполнялся из-за ограничения сетевой
активности только VM/ns. Старый helper `sleep infinity` не использовался.

Это подготовленный **public-protection gate**, не приёмка system-profile.
Не подтверждены actual production VM lifecycle, TUN traffic, dedicated positive
nonroot UID, worker-first cleanup, foreign sentinels и полный cleanup VM.
P09/P12/P13 остаются partial. RED→GREEN production поведения не заявляется.
Локальные логи проверок:
`/home/myrs/projects/yonik/local/validation/production-helper-vm/`.

Локально `cargo test --offline -p yonik-linux --test installed_helper_session`:
**1 passed, 2 ignored**. Прямой запуск ignored VM case на host ожидаемо завершился
отказом «Нет явного VM opt-in» до D-Bus/effects (`host-refusal.log`).
Ruff check/format и rustfmt целевых файлов прошли. Свежие helper (114 MiB), worker
(92 MiB) и libtest сохранены в постоянном `artifacts/` рядом с
`binary-provenance.json`; production sources соответствуют базе, test source
зафиксирован отдельным SHA256. Cargo 1.85.1 изменил только сортировку lockfile;
семантическое сравнение package/version/dependencies подтвердило эквивалентность,
изменение lockfile исключено из patch.

Clippy с `-D warnings` остановился на существующем `collapsible_match` в
`crates/yonik-engine/src/lib.rs:165` (вне ownership, не исправлялся).
Целевой `cargo clippy --offline --no-deps -p yonik-linux --test installed_helper_session -- -D warnings`
прошёл. Полный workspace clippy PASS не заявляется.
