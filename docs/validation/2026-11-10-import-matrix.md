# P04: детерминированная матрица ограниченного импортера

Base: `21aba59f7e161a67d98ebe7415315e3fb8afcf9a`.
Работа выполнена в отдельном worktree; system-session-runtime из основного
checkout не читался и не переносился. Изменения parser и зависимостей не нужны:
новые случаи прошли на существующей реализации.

Основание: [контракт импорта](../specs/data-and-contracts.md), FR-06/FR-07,
[матрица приёмки](../specs/validation-and-roadmap.md), P04 в PLAN.md.
Проверки используют только публичный API `import_bytes`, без property/fuzz-зависимостей.
Код: [matrix.rs](../../crates/yonik-import/tests/matrix.rs).

| Область | Входы | Проверяемый внешний результат |
| --- | --- | --- |
| Размер отдельной URI | 64 KiB − 1, ровно 64 KiB, 64 KiB + 1; пароль UTF-8; plain и Base64 | Лимит в байтах; первые два приняты, превышение даёт TooLarge/Source на строке 1; соседняя запись сохранена |
| Размер всего входа | 8 MiB − 1, ровно 8 MiB, 8 MiB + 1; JSON и Base64 JSON с пробелами | Лимит до удаления пробелов/декодирования; превышение TooLarge, полные исходные байты доступны |
| Число записей | 19 999, 20 000, 20 001 неизвестных URI; пустые строки между ними; plain/Base64 | Отклонённые непустые строки тоже расходуют бюджет; до предела UnsupportedProtocol и физические индексы; выше TooManyRecords |
| Глубина JSON | 63, 64, 65 чередующихся объектов/массивов; строка со скобками, backslash и экранированной кавычкой; plain/Base64 | До 64 native JSON сохранён дословно; 65 даёт JsonTooDeep с сохранением входа; скобки строки не расходуют бюджет |
| Base64 | Standard/URL-safe, с padding/без, ASCII whitespace каждые 7 символов | Один слой принимается, decoded URI сохраняется дословно; второй envelope даёт UnsupportedFormat |
| Повреждённый Base64 | Неполная группа A, лишний padding Zg===, неканонические trailing bits Zh==, невалидный UTF-8 /w== | InvalidEncoding либо InvalidUtf8; исходные байты доступны |
| IDNA/IP | Кириллица uppercase, готовый punycode, IPv4, развёрнутый IPv6, IPv4-mapped IPv6; Trojan URI и VMess JSON | Одинаковые нормализованные endpoint и port для двух входных форм |
| Опасные/неоднозначные URI | IPv6 zone ID, повреждённый IPv6, неверный punycode, port 65536, LF в credentials, неверные UTF-8/percent escapes, дубли известного ключа после decoding и неизвестного ключа | Нет принятого опасного профиля; одна issue на физической строке 3, соседний валидный профиль и весь исходник сохранены; plain/Base64 |
| Неизвестные опции | VLESS, Trojan, SS, HTTP CONNECT, SOCKS5: Unicode key, escaped delimiters и allowInsecure=1 | Исходник и query побайтно сохранены; UnknownParameter и UnsafeOption; Debug скрывает canary |
| Неизвестные VMess поля | Вложенный объект/массив неизвестной опции | Decoded JSON сохранён дословно в parameters; UnknownParameter |

Существующие тесты дополнительно проверяют bounded reader, неполный исходник при
IO/overflow, BOM, SIP002/legacy SS, шесть протоколов, JSON-дубли, managed Xray,
identity и отказ активации неподдерживаемых опций. Новая матрица не объявляет
ParsedServer разрешением на активацию; allowInsecure сохраняется с review flag.

## Проверки

Независимое review выявило, что исходная Base64-строка занимала 54 UTF-8 байта
и не требовала padding. Тест расширен тремя длинами по модулю 3 (0, 1, 2):
для Standard/URL-safe явно проверяется ровно 0, 2 или 1 завершающих `=`,
для NO_PAD — их отсутствие. Для каждого случая проверяются импорт, whitespace
и отказ от повторной обёртки. Production parser не менялся.

Rust 1.97.0, Cargo 1.97.0, Linux. `CARGO_BUILD_JOBS=2`.
Cargo target: `/var/tmp/codex-myrs/transient/yonik-import-matrix-b43d5d8e/target`,
все Cargo test/clippy/clean удерживали shared flock корня task-каталога.

- `cargo test --locked -p yonik-import`: 64 integration tests и 1 doctest прошли.
- `cargo fmt -p yonik-import -- --check`: прошло.
- `cargo clippy --locked -p yonik-import --all-targets -- -D warnings`: прошло.
- Cargo.lock после первого запуска восстановлен: Cargo только переставил записи;
  окончательные проверки выполнены с исходным lock и `--locked`.
- Desktop, Docker и другие crates не собирались; сетевых запросов parser не выполнял.

## Конкретный остаток P04

Это ограниченный срез P04, не завершение пункта. Остаются property/fuzz-кампании
с корпусом повреждений, более широкая матрица URI-диалектов/транспортов по
согласованным capabilities, IDNA edge cases и неоднозначных числовых IP/префиксов,
проверки ручной формы и полной цепочки импорта/подтверждения сохранения в GUI.
Managed/native JSON требует отдельной матрицы поддерживаемых Xray/sing-box полей
и согласования принадлежащих приложению сетевых настроек.

32 MiB decoded — защитный внутренний потолок, недостижимый через публичный
8 MiB input при одном слое Base64 без распаковки. Эта работа не заявляет
проверку его точной границы и не ослабляет input cap ради такого теста.
MSRV, Android и workspace-wide проверки в этом срезе не запускались.
Независимое review точных base/head SHA и CI требуются перед merge.
