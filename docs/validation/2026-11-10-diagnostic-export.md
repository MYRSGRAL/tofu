# Безопасные диагностические метаданные Linux

База: `f8975948406c920c4d78439348001394647d17dd` (проверены HEAD и remote main).
Это частичная проверка FR-16 / L20a, не полный PASS P11/P25.

## Фактическая поверхность

`tools/ui_benchmark.py` пишет evidence.json, счётчики stderr и локальные
технические ошибки; `tools/desktop_smoke.py` пишет results.json и fixture scripts.
`tools/engine_compile.py` создаёт native-конфиги fixture. Это внутренние инструменты,
а не публичный продуктовый API безопасного экспорта. Они не изменены.
`DesktopError::key` в `crates/yonik-desktop/src/backend.rs` возвращает ключ,
RU/EN находятся в `Main.qml`. Единого диагностического DTO/export API нет.

## Реализованный срез

`python3 tools/check_diagnostic_export.py < metadata.json > safe-metadata.json`
читает максимум 65537 байт stdin; принимает до 65536, выдаёт JSON только после
полной проверки. Код отказа 2, stderr фиксированный, без входных данных.
CLI не читает файлы приложения, архивы, keyring или системные журналы.
При shell redirect отказ может оставить пустой выходной файл.

Схема v1 (все поля обязательны, неизвестные запрещены на каждом уровне):

```json
{"schema_version":1,"platform":"linux","errors":[{"code":"CANCELLED","message_key":"cancelled","stage":"operation","retryable":false,"params":{}}]}
```

Обоснование allowlist по актуальным контрактам:

- `schema_version`: ровно integer 1; явная версия нового среза.
- `platform`: ровно linux; Android вне задачи.
- `errors`: максимум 128 записей. §5 data-and-contracts требует стабильный код,
  message-key, этап, повторяемость и безопасные параметры.
- `code` / `message_key`: только согласованные пары CANCELLED/cancelled и
  CLEANUP_FAILED/cleanup_failed. Это подмножество существующего QML-каталога;
  тест сравнивает обе RU/EN строки с текущим каталогом.
- `stage`: только operation, без неподтверждённой детализации этапа.
- `retryable`: только bool, семантическое решение остаётся за producer.
- `params`: только пустой объект. Безопасной схемы произвольных параметров и
  native cause сейчас нет, поэтому они не экспортируются.

Глубина до 4 проверяется до JSON parser; размер до разбора; duplicate keys,
NaN/Infinity, неверный UTF-8, float вместо integer и integer вместо bool запрещены.
Строковые значения ограничены конечными перечислениями. Нет свободных строк,
идентификаторов пользователя, путей, URL, timestamps, native JSON или raw source.
Credential canaries и управляющие C0/DEL/C1/ESC отклоняются через схему,
без эвристического regex redaction. §6 data-and-contracts и FR-16/раздел 3
requirements запрещают утечку секретов через диагностический экспорт.

## Проверка и остаток

Targeted RED: импорт нового модуля отсутствовал; GREEN: новые regression tests.
Штатный CI уже вызывает `python3 -m unittest discover -s tools/tests -v`;
тест размещён в `tools/tests`, исходный ownership `tests/tools` исправлен без
изменения workflow. Ruff format/check применены к двум Python-файлам.
Rust не менялся, cargo/fmt/clippy Rust не запускались; build/transient не создавались.
Evidence полного tools discovery вне TTL:
`/home/myrs/projects/yonik/local/validation/diagnostic-export-tests.log`.

Остаётся подключить producer приложения к этому срезу, согласовать расширение
кодов/этапов/типизированных параметров и UI экспорта. CLI не преобразует внутренние
логи и не обещает безопасное архивирование пользовательских файлов. До такого
подключения безопасный экспорт из приложения не считается реализованным.

Финальный результат: штатный discovery **132 tests OK**, из них 7 новых;
Ruff format/check — PASS, staged diff check — PASS.
