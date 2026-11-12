# Format4: normal GUI и настоящий Secret Service

Production source: `cceede520842a17e4ef548807f63f64b2be19226` (accepted main).
Production-файлы, bridge, API, Cargo.toml/lock и зависимости не изменены.
Normal binary собран `cargo build --locked -p yonik-desktop --features gui
--bin yonik-desktop`, Rust 1.97.0, host Qt 6.11.2. Суффикс `dirty` в заголовке
относится к добавленным незакоммиченным acceptance-файлам; SHA256 всех tracked
исходников до запуска GUI сохранены в source-manifest.json.

## Наблюдения

Один узкий actual lifecycle, без повторения принятой PR118 Secret Service
матрицы и без traffic-проверок:

1. Fresh normal GUI PID 5494, обычный HostConfig, DB3, один Secret Service item.
2. Внешним X11 input введён fixture text, открыта проверка импорта: принято 1,
   отклонено 0. Через настоящий SaveFile dialog создана копия
   `storage/copies/backup3.sqlite`. GUI сообщил «Резервная копия готова».
   До approve обе БД имели user_version=3 и integrity_check=ok.
3. «Сохранить профили» повысило live DB до format4. Через OpenFile dialog
   выбран отдельный file fixture, review: принято 1, отклонено 0; второй approve
   сохранил второй source. Backup остался format3.
4. GUI завершён через штатное меню трея «Выйти и отключиться»; процесс исчез.
   Отдельный storage witness подтвердил оба original и revision=1.
5. Новый normal GUI PID 6332 открыл сохранённые text и file профили. Оба
   результата просмотрены на экране через выбор источника. Второй штатный выход,
   повторный storage witness: original_matches=true, revision_matches=true,
   count=2. Это supplemental storage witness, а не чтение original через GUI.

На протяжении lifecycle item остался
`/org/freedesktop/secrets/collection/login/1`, count=1, locked count=0.
Каждый inspect сверял owner Unix PID с PID единственного управляемого
foreground gnome-keyring. После завершения daemon имя исчезло с private bus.
Verifier использует `subscription_master_key(false)`, repository и
`SubscriptionService::offline`; не принимает ключ от caller, не создаёт ключ,
не выводит plaintext, ключ, hash ключа или ciphertext.

## Изоляция

Одноразовая QEMU VM на официальном Arch image `20261101.620452`. Перед boot
перепроверены image/public-key/signature SHA256 и GPG VALIDSIG primary fingerprint
`1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F` по сохранённому provenance PR118.
Образ read-only, его корень под shared BSD flock; собственный overlay и target
под lock корня task. Private dbus-daemon config без service directories;
до явного запуска keyring имя отсутствовало. Пароль создан только внутри VM,
файл 0600, передан daemon через stdin. В evidence пароля нет.

GUI работает как пользователь fixture внутри Xvfb + Openbox + tint2. Все
xdotool events направлены только в эту private X11 session. Host desktop,
Secret Service и настройки окружения не использовались для GUI/credential flow;
установка пакетов выполнялась только в VM. Guest Qt 6.12.0, gnome-keyring
1:51.1-1, dbus 1.16.2-2; полный список сохранён в packages.txt.

## Диагностика выбора backup

Ранние отказы сохранены, они не выдаются за successful backup. Ввод абсолютного
пути в поле filename Qt dialog при cwd=/root сформировал
`/root//home/fixture/storage/backup3.sqlite`. Внешний GDB остановился только на
вызове DesktopHost::dispatch для maintenance_start и вывел этот несекретный
запрос с revision=0. Ключи/память repository не читались. Затем выбор live
data directory встретил его instance flock и дал busy. Нормальная навигация
Home → storage → copies, ввод basename и Save дали verified backup. Production
не исправлялся. Не следует повторять прежнюю гипотезу, будто pending local
review сам по себе запрещает backup: успешный backup выполнен при этом review.

## Evidence и воспроизведение

Полные binaries/manifests/logs/screenshots сохранены вне TTL:
`/home/myrs/.t3/worktrees/yonik/t3-af487ac0-310beaee221833d5/local/validation/secret-format4-gui/`.
Публичная сводка и hashes: `docs/evidence/format4-secret-service-20261112.json`.
Source manifest создан после VM bootstrap и до первого GUI launch; итоговый
artifact manifest дополняет его hashes binaries, harness и logs. Это не
утверждение, что весь итоговый bootstrap runner повторно запускался: Openbox,
tint2 и диагностический GDB установлены дополнительными guest-only командами;
соответствующие package logs сохранены. Финальный runner включает Openbox/tint2.

Основные screenshots: 04-text-review, 15-backup3-ready, 18-file-review,
19-file-approved-format4, 20-production-quit-menu, 21-new-process-reopen,
22-new-process-file. Их визуально проверил implementer. Input с credentials
на screenshot не снимался. В observations.jsonl сохранены и ранние отказы;
offline verifier использует последнее наблюдение каждого именованного stage.

Инструменты fixture-only: format4_gui_vm.py поднимает VM; format4_gui_guest.py
управляет только её собственной сессией; format4_gui_control.py передаёт
операции по SSH; format4_gui_verify.py проверяет публичные gates.
operations.jsonl сохраняет X11 input без payload (text читает guest fixture).
Для повторения нужны новый task root, read-only provenance cache, QEMU/KVM,
guest network для pacman, host Qt/Rust. CLI control привязан к default root
этой задачи; при другом root нужно согласованно задать его в host control.

Rust build/check выполнялись через with-rust-slot, jobs=1,
`CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/secret-format4-gui/target`.
GUI smoke flags, persistence key injection, test-only bridge hooks не применялись.
Аппаратный login/PAM, KDE Wallet, GUI unlock prompt и старый reader backup этим
узким lifecycle не проверяются; их не следует выводить из данного результата.

## Итоговые проверки и очистка

Actual GUI build и build verifier PASS. Offline acceptance gates PASS; три
unittest (включая восемь отрицательных подслучаев) PASS. Ruff check/format и
cargo fmt --check PASS. Новый verifier отдельно прошёл
`cargo clippy --locked --no-deps -p yonik-desktop --example
format4_custody_witness -- -D warnings`.

Полный `cargo clippy --locked -p yonik-desktop --features gui --all-targets
-- -D warnings` RED: Rust 1.97 сообщает clippy::collapsible_match в неизменённом
accepted `crates/yonik-engine/src/lib.rs:170`. Production не исправлялся,
подавления lint в исходники не добавлялись. Лог отказа сохранён; общий clippy
не выдаётся за PASS.

После проверки сохранённых witnesses VM не перезапускалась. Перед очисткой
получен exclusive lock корня task, проверено отсутствие активных cwd/exe/fd/maps
ссылок на него. `cargo clean` удалил 8740 файлов (4.6 GiB); собственный root
с overlay, SSH fixture key и guest password удалён. Нужные binaries и evidence
остались в workspace вне TTL, 373 MiB. `cleanup.json` и `clean.log` фиксируют
результат; `final-manifest.json` содержит итоговые hashes, сохраняя прежний
artifact-manifest.json без переписывания.
