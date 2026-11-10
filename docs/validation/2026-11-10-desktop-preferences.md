# Постоянные настройки desktop — ограниченный срез P19

Дата: 2026-11-10. Репозиторий: yonik. Ветка: `t3/persist-desktop-preferences`.
Фактическая база: `e33fa1908dfb3deb5434506341674de0b3a827b4`.
Head реализации: `7a86eb9`. Итоговый head с evidence указан в отчёте PR.

## Контракт и изменения

Изменены только desktop backend, объявление preferences в lib, QML и тесты.
Новый `src/preferences.rs` хранит JSON версии 1, максимум 4096 байт, в
`preferences.json` внутри приватного data dir. Используется существующий
exclusive flock каталога backend, файл 0600, NOFOLLOW/NONBLOCK при чтении,
create_new для временного файла, fsync и атомарная замена. Первая публикация
использует hard_link без замены существующего имени. Содержимое проверяется
перед заменой; обнаруженное внешнее изменение запрещает запись. Чужой temp
не удаляется. В модели безопасности каталог 0700 принадлежит текущему UID;
flock координирует экземпляры приложения, а не враждебные процессы того же UID.

Сохраняются engine, фиксированный порт либо automatic (`null`), безопасный
health endpoint и ожидаемый статус, язык RU/EN. Timeout остаётся существующей
константой 5 секунд: API его изменения нет. Новые controls не добавлены.
`reset_listen_port` возвращает существующий backend API к automatic. CLI port
остаётся явным override. Эфемерный выбранный automatic-порт не сохраняется.

Язык QML теперь изменяется через `set_language`, восстанавливается из snapshot.
Повреждённый, усечённый, неизвестный, слишком большой или небезопасный файл
оставляется на месте; работают defaults, уведомление в настройках и запрет
перезаписи. Ошибка до публикации не применяет запрошенные значения в памяти.
Исправление post-publication outcome и ошибок appearance описано в
[отчёте review](2026-11-10-desktop-preferences-review.md).

Подписки и ключи не копируются. Health URL с query/fragment остаётся только в
памяти до выхода; GUI сообщает об этом, на следующий запуск сохранён стандартный
endpoint/status. Userinfo отклоняет существующий ProbeTarget. Это сохраняет
старый контракт URL с секретным query без записи токена в plaintext; сохранение
таких адресов требует отдельного зашифрованного контракта и остаётся за срезом.

Настройки подключения нельзя менять при удерживаемой сессии, включая
CleanupError. Connect передаёт session task отдельные compiled config и
ProbeTarget; язык не меняет сетевые параметры. LocalSession и P18 layout сохранены.

## Проверки и evidence

Все Cargo-команды: `+1.85.1`, `CARGO_BUILD_JOBS=2`,
`CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/yonik-preferences/target`;
каждый запуск под `flock -s /var/tmp/codex-myrs/transient/yonik-preferences`.
Логи и trace сохранены рядом в `2026-11-10-desktop-preferences/`, вне TTL.

- `cargo test -p yonik-desktop --locked`: 11 unit и 27 integration passed;
  один отдельный namespace network test штатно ignored.
- `cargo test --workspace --locked`: 529 passed, 0 failed, 24 ignored
  (изолированные namespace/engine/host-проверки штатно требуют отдельных runners).
- `cargo clippy -p yonik-desktop --features gui --all-targets --locked -- -D warnings`: passed;
  C++ Qt/GCC 16 печатает предупреждения системных заголовков, полный вывод в clippy.log.
- `ruff format --check` и `ruff check` нового Python smoke: passed существующим
  ruff 0.16.6 из локального uv cache, без установки пакетов.
- `cargo build -p yonik-desktop --features gui --locked`: passed.
- `python3 tools/desktop_ui_smoke.py <binary> --output <evidence>/ui`:
  passed, 142 действия, 3 TCP EOF, 7 HWID-запросов, отмена HTTP;
  synthetic loopback fixtures, offscreen/software Qt, без системной сети/ядер.
- `python3 crates/yonik-desktop/tests/preferences_smoke.py <binary> <evidence>/reopen`:
  passed, два запуска Qt восстанавливают engine, endpoint/status и язык.
  Smoke использует новый ephemeral key на процесс: во второй data dir переносится
  только preferences, без БД. Rust integration отдельно открывает одну БД повторно
  с постоянным тестовым ключом.
- `python3 tools/check_translations.py`: 164 inline RU/EN пары и 77 ключей passed.
- `cargo fmt --all -- --check`, `git diff --check`: passed.

Первые диагностические запуски smoke выявили неподдерживаемый key event на
корневом Window и новый ephemeral key между процессами; исправлен только тестовый
сценарий. Первый Cargo запуск без --locked переставил workspace entries lockfile;
исходный Cargo.lock восстановлен, зависимости не менялись.

Строгий RED-before-implementation порядок TDD в этом срезе не подтверждён:
первые запуски новых Rust-тестов уже прошли. Тесты проверяют поведение, но
отдельного доказательства RED нет; это ограничение процесса, не заявление TDD.

## Остаток

P19 целиком не принят: системная сессия, recovery, управление версиями и общая
продуктовая матрица остаются вне ownership. Сохранение URL с потенциальными
секретами в параметрах ограничено текущим процессом. Независимое review и merge
выполняет root. Host helper, keyring, пользовательская сеть и подписки не менялись;
Docker/deploy/release не запускались.

## Очистка

Активных процессов со ссылками на свой target перед очисткой не обнаружено.
`cargo +1.85.1 clean` удалил 12431 файл / 7.2 GiB; собственный transient root
удалён через rmdir. Логи, результаты и trace сохранены в репозитории;
сгенерированные PNG оставлены вне Git и TTL:
`/home/myrs/.t3/artifacts/yonik-desktop-preferences-20261110`.
