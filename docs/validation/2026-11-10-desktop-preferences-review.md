# Исправления независимого review PR #52

База review и исправлений: `a6d6fe2b224d9ea93fdd9777c2e0200a4b0a0e87`.
Ветка: `t3/persist-desktop-preferences`. Итоговый head указан в ответе и PR.
Срез: P2(1), P2(2), P3; P19 целиком не объявляется принятым.

## Изменения

- QML appearance показывает `preferences_write` в `appearanceSaveError`.
  При отказе смены языка ComboBox возвращается к подтверждённому snapshot.
  RU/EN подсказка объясняет сохранение языка после перезапуска.
- До публикации `Store::save` возвращает ошибку и backend не применяет изменение.
  После успешного rename/hard_link возвращается `SaveOutcome::Durable` либо
  `PublishedDurabilityUnknown`. Backend принимает опубликованные настройки,
  синхронизирует память с видимым файлом и выставляет отдельное предупреждение.
  GUI явно говорит, что при сбое питания сохранность неизвестна и отката нет.
- Порядок durability сохранён: запись temp → fsync файла → публикация → fsync
  каталога. Ошибка post-publication не превращается в ложное «не применено».
  Удаление собственного temp после hard_link тоже не объявляет опубликованное
  значение отклонённым. Следующее сохранение сверяется с опубликованным original.
- Никаких изменений application.cpp/ui_smoke.h/tray, зависимостей, keyring,
  пользовательских подписок или сети. Защита damaged/unknown/symlink, ограничения
  URL с параметрами и immutable параметры активной сессии сохранены.

## RED → GREEN

`preferences::tests::directory_sync_failure_reports_publication_and_reopens_new_settings`
инъецирует ошибку только финальной синхронизации каталога, после успешной публикации.
RED: reopen уже прочёл `en`, но старый save вернул Err; тест упал с сообщением
«опубликованное значение нельзя объявлять отклонённым» (`red.log`). GREEN проверяет
различимый outcome, reopen `en`, затем следующую успешную запись и reopen `ru`.
Backend-тест отдельно проверяет согласованность preferences/state/reopen при
том же fault и наличие предупреждения; проверки запрета изменения session settings
остались в тесте.

QML RED: новый smoke на старом экране после ready создал конфликтующий temp,
backend выдал `preferences_write`, сценарий остановился на ожидании видимого
appearanceSaveError (`qml-red.log`, trace). GREEN проверяет реальные QML-объекты:
видимый понятный текст, прежний язык и индекс ComboBox. Для changed-original и
конфликта temp проверяется побайтовое сохранение чужого файла. Инъекции выполняются
только внутри собственных TemporaryDirectory на синтетических данных, после ready.

## Проверки

Cargo: MSRV `+1.85.1`, `CARGO_BUILD_JOBS=2`, target
`/var/tmp/codex-myrs/transient/yonik-preferences-review/target`; каждый запуск
под flock корня `/var/tmp/codex-myrs/transient/yonik-preferences-review`.
Логи и trace находятся в `2026-11-10-desktop-preferences-review/` вне TTL.

- `cargo test -p yonik-desktop --locked`: 12 unit + 27 integration passed,
  отдельный namespace-тест штатно ignored.
- `cargo fmt --all -- --check`: passed.
- Ruff check/format --check `crates/yonik-desktop/tests/preferences_smoke.py`: passed.
- `python3 tools/check_translations.py`: 165 RU/EN inline-пар и 77 ключей passed.

- `cargo test --workspace --locked`: 530 passed, 0 failed, 24 штатно ignored.
- `cargo build -p yonik-desktop --features gui --locked`: passed.
- `cargo clippy -p yonik-desktop --features gui --all-targets --locked -- -D warnings`:
  passed; сохранились предупреждения C++ из системных Qt/GCC 16 headers.
- `python3 crates/yonik-desktop/tests/preferences_smoke.py <binary> <output>/qml-green`:
  passed, reopen и оба runtime fault-сценария. QML-снимок ошибки просмотрен:
  сообщение полностью видно и не обрезано, ComboBox показывает «Русский».
- `python3 tools/desktop_ui_smoke.py <binary> --output <output>/ui`:
  passed, 142 действия; 3 TCP EOF, 7 HWID-запросов, подтверждённая отмена HTTP.

Синтетические GUI запущены offscreen/software, без системной сети и ядер.
Перед очисткой процессов из своего target не обнаружено; `cargo +1.85.1 clean`
удалил 12431 файл / 7.2 GiB, transient root удалён через rmdir.
Логи/trace/results сохранены в Git. PNG вне Git и TTL:
`/home/myrs/.t3/artifacts/yonik-desktop-preferences-20261110/review`.

Merge/watch/delegation не выполняются; независимый повторный
review ограничивается diff от указанной базы и затронутыми контрактами.
