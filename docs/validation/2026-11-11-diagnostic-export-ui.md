# Подключение безопасного экспорта к Linux UI

База: `565fa26bd745565dbe64a47af8d970b6e57a3073`.

В разделе «Настройки → Диагностика» добавлено явное действие «Копировать
диагностику» / “Copy diagnostics”. Нажатие вызывает Rust producer через CXX-Qt;
только успешный результат передаётся QClipboard. Автоматического копирования нет.
Python в приложении не запускается.

Producer создаёт новый schema1 JSON и читает только поля ошибок snapshot:
`error`, `connection_error`, `source_error`, `routing_ack.error`.
Разрешены пары CANCELLED/cancelled и CLEANUP_FAILED/cleanup_failed;
`stage=operation`, `retryable=false` (автоматический повтор не обещается),
`params={}`, `platform=linux`. Дубликаты удаляются. Неизвестная ошибка или
неверный тип приводят к фиксированному отказу, clipboard сохраняется.
Остальные поля snapshot не экспортируются. Вход ограничен 1 MiB, ошибки —
четырьмя выбранными полями, выход — менее 1 KiB. История ошибок не собирается.

Схема и старый Python-валидатор сохранены. Rust regression отправляет оба кода
в существующий валидатор; Python tests проверяют соответствие RU/EN каталогу.
Canary-тесты проверяют URL/credentials/native source и диапазоны C0/DEL/C1.
QtTest проверяет явное нажатие и обе локали; smoke полного приложения проверяет
реальный producer и содержимое clipboard посредством вставки в существующий
редактор, без отправки данных или сетевой операции.

Команды целевых проверок:

```sh
cargo test --locked -p yonik-desktop --lib diagnostics
cargo build --locked -p yonik-desktop --features gui
cargo clippy --locked -p yonik-desktop --features gui --all-targets -- -D warnings
python3 -m unittest discover -s tools/tests -p test_check_diagnostic_export.py -v
QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software dbus-run-session -- /usr/lib/qt6/bin/qmltestrunner -input tools/tests/qml
```

Cargo выполняется через общий with-rust-slot, jobs=1, с собственным внешним
CARGO_TARGET_DIR и flock корня task-каталога. Логи, smoke-сценарий и trace:
`/home/myrs/projects/yonik/local/validation/diagnostic-export-ui/` (вне TTL).

Это узкий срез FR-16: полный P11/P25 не заявляется. Неизвестные коды пока
делают экспорт недоступным; свободные параметры и native cause не добавлены.

Проверенные результаты: Rust targeted **3 PASS** (вложенная ошибка сначала
поведенчески RED, затем GREEN); Python validator **7 PASS**; QtTest **3 PASS**.
Полное приложение: offscreen/private D-Bus smoke с RU/EN, неизвестной командой,
фиксированным отказом и проверкой сохранения clipboard — exit 0, 13 шагов.
Сценарий сохранён в `tools/tests/qml/diagnostic-export-smoke.json`.

`cargo test --features gui --lib` не линкуется: bridge находится в binary,
а generated CXX initializers требуют его символы. Этот срез не меняет устройство
GUI targets: unit tests запускаются без gui, настоящий GUI проверяется binary.
Первый smoke с длинным XDG_RUNTIME_DIR завершился отказом до загрузки окна
(лимит Unix socket path); успешный использовал собственный короткий каталог
`/tmp/yonik-diag-995025`, без доступа к пользовательскому clipboard.

Clippy desktop `--features gui --all-targets --no-deps -- -D warnings` — PASS.
Полный вариант без `--no-deps` остановлен существующим `collapsible_match`
в `crates/yonik-engine/src/lib.rs:165`; этот файл вне scope и не изменён.
`cargo fmt --all --check` и `git diff --check` — PASS.

Финальная GUI-сборка и повторный smoke (13 шагов) — PASS.
Проверенный stripped binary сохранён рядом с evidence как `yonik-desktop`.
Собственный Cargo target очищен штатно: удалено 9116 файлов / 4,6 GiB;
transient task-каталог и собственный Qt runtime удалены.
