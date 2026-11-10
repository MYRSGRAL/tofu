# Linux P20/P15: каталог приложений и кандидаты процессов

База: `55e23c26295944a64451b4a2a72b0b404bf7cb3f`. Итоговый SHA — commit,
содержащий этот отчёт (`git log -1 --format=%H -- crates/yonik-linux/src/applications.rs`).

Публичный модуль `yonik_linux::applications`: `Catalog::new`,
`desktop_snapshot`, `process_snapshot`, `process_is_current`; `Limits`,
`Snapshot<T>`, `DesktopApplication`, `ProcessCandidate`, `Unsupported`.
Вызовы синхронные, по запросу; таймеров, watchers, фоновых задач нет.

Вызывающий код передаёт абсолютные applications-каталоги в порядке
XDG_DATA_HOME, XDG_DATA_DIRS (обычно ~/.local/share/applications,
/usr/local/share/applications, /usr/share/applications), proc root и LC_MESSAGES.
API не читает окружение. Корни без symlink-компонентов; относительные пути,
parent-компоненты, невалидная locale и чрезмерные лимиты отклоняются.

Desktop ID получается заменой / на - в относительном пути. Первый ID маскирует
последующие, включая Hidden и невалидные записи. Type!=Application исключается.
NoDisplay сохраняется как атрибут, не теряет установленное приложение.
Локализованный Name: country@modifier → country → language@modifier → language
→ обязательный Name, encoding игнорируется. Строковые escape декодируются.
Icon возвращается как данные, файл значка не открывается.

Exec не исполняется, argv не возвращается. Поддерживается консервативный
поднабор: абсолютный путь без кавычек и escape, обычный исполняемый ELF-файл,
без symlink-компонентов. PATH не используется. Wrapper/interpreter, Flatpak,
Wine, AppImage, D-Bus activation, сложный Exec и неподтверждённые пути имеют
явный Unsupported. Проверка ELF magic подтверждает формат-кандидат, а не
целостность бинарника или отсутствие wrapper-логики внутри ELF. Путь может
измениться после снимка: перед применением политики нужна повторная проверка.
Кавычки и часть допустимых freedesktop-команд намеренно неподдерживаемы.

Ограничения по умолчанию: 128 каталогов, глубина 8, 8192 обследованных имён,
64 KiB на файл, 4096 результатов. Есть жёсткие верхние границы параметров.
Обход и результаты ограничены; сортировки O(n log n), разбор O(bytes).
Чтение через openat с NOFOLLOW/NONBLOCK и проверкой regular file; каталоги
закреплены FD, перечисление через /proc/self/fd требует доступного procfs.
При достижении лимита или недоступном файле incomplete=true. Неполная выборка
не обещает одинакового подмножества при изменяющемся порядке readdir.

Процессы: только PID/starttime/name/exe, stat перечитывается после exe,
exe также проверяется повторно. PID не является постоянным селектором.
Исчезнувший/изменившийся процесс исключается, недоступный exe даёт None и
incomplete. Deleted exe не предлагается. Между проверкой и использованием
возможна смена процесса; API не обещает атомарный снимок или pidfd identity.
cmdline, environ, сокеты и секреты не читаются. exe процесса — диагностический
путь, не результат desktop mapping и не автоматически подтверждённое правило.

Источники прочитаны до реализации парсера:
[Desktop ID](https://specifications.freedesktop.org/desktop-entry/latest/file-naming.html),
[локализация](https://specifications.freedesktop.org/desktop-entry/latest/localized-keys.html),
[ключи](https://specifications.freedesktop.org/desktop-entry/latest/recognized-keys.html),
[Exec](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html).

Это отдельный срез данных: GUI, реальные TCP/UDP attribution, packet routing,
проверка native engine capabilities и Android отсутствуют. P20 целиком не закрыт.

## Проверки

Результаты команд и ограничения проверки добавлены ниже перед commit.

Тесты используют отдельные временные каталоги и не запускают Exec: hidden masking,
localized precedence, NoDisplay, Type, nested ID, symlink loop/device, настоящий
FIFO, размер файла/число результатов, неподдерживаемые команды, проверка native
пути, stat с пробелами/скобками в имени, устаревший starttime и исчезнувший stat.
Первые тесты дали RED (E0432: публичного applications ещё не было), затем GREEN.
Ограничение: нет детерминированной инъекции смены stat непосредственно между
двумя чтениями, отдельного теста EACCES и полного перебора всех лимитов.

Реализация: `ee6261f` (feat(linux): добавить ограниченный каталог приложений и процессов).

Проверено на Rust 1.85.1, CARGO_BUILD_JOBS=2:

- `cargo +1.85.1 fmt --all -- --check`: exit 0.
- `cargo +1.85.1 clippy -p yonik-linux --all-targets -- -D warnings`: exit 0.
- `cargo +1.85.1 test -p yonik-linux`: exit 0, 364 passed.
- `cargo +1.85.1 test --workspace --exclude yonik-desktop`: exit 0, 512 passed, включая 8 catalog tests.

Qt desktop-пакет не проверялся; изменения не затрагивают GUI.
Зависимости не добавлены, Cargo.lock сохранён без изменений.
Evidence сохранён в этом отчёте вне TTL до cargo clean.
