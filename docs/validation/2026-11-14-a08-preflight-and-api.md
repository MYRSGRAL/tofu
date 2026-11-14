# A-08: source preflight и предложение root

База: `1461774556459d2313fcb73fb6ac725df371fb4d` (accepted main #188/#190).
Исходная ветка `docs/n5-minimal-evidence-contract` / `7f18d6b` сохранена.
Новая task-ветка не переписывает историю. Общие production-файлы не изменены.

**A-08 OPEN.** Это source preflight и конкретное предложение наблюдаемости,
не GUI приёмка. FR-20/G-APP, native admission и physical Linux не закрыты.

## Свежий результат

`tools/a08_retained_binding.py RETAINED OUTPUT` проверяет сохранённые ELF/archive
P03 по accepted `p03-integrated-bindings.json`, затем сравнивает все tracked crates
и Cargo manifests текущего checkout с архивом без извлечения файлов. Это
консервативная надмножество closure, а не определение linker dependency graph.
Отсутствующие, повторные и symlink archive members не принимаются.

Результат: [retained-preflight.json](../evidence/a08/retained-preflight.json).
Integrity двух retained inputs совпала; 33/387 paths отличаются. Среди них
`crates/yonik-desktop/src/backend.rs`: даже без unrelated examples отказ
переиспользования сохраняется. Exit **2** — ожидаемое отклонение, не PASS runtime.
P03 retained inputs прочитаны по месту, не изменены и не скопированы. Старые
runtime logs не использованы как новое доказательство. Новый ELF не строился;
build/transient каталоги и процессы не создавались.

Тест gate содержит positive equality и четыре negative cases: изменение bytes,
missing member, duplicate member, symlink member. Тесты проверяют только
source gate, а не parser, proc reads или GUI. Их свежий лог и hashes находятся
в `docs/evidence/a08/`.

## Локализованный разрыв контракта

Прочитаны A-08 в `validation-and-roadmap.md`, существующие G-APP/catalog reports
и P03/P07 fixture pattern. Доступный source path на этой базе:

- `Main.qml` routingPanel: checkbox процессов → «Загрузить каталог» →
  `window.send({command: "routing_catalog", processes, query, offset})`.
  Native capability для отображения этих controls не требуется.
- `backend.rs::Command::RoutingCatalog` содержит только processes/query/offset;
  handler ждёт `spawn_blocking(editor::catalog)` и записывает общий routing_catalog.
- `editor::catalog` создаёт настоящий `Catalog`, выбирает desktop/process snapshot,
  возвращает incomplete и страницы по 32. `.desktop` корни включают системные
  пути; `XDG_DATA_HOME` добавляется первым, `/proc` задан явно.
- `applications::process` читает stat/exe дважды, не cmdline/environ.
  Это source observation, а не syscall oracle и не доказательство чужого UID.
- `PlatformSourceInput` — закрытый serde enum с deny_unknown_fields;
  `Main.qml::isUnsupportedSelector` сравнивает ровно `unsupported_selector`.
  Неизвестный kind parser refusal и известный Android selector на Linux —
  разные случаи; generic `Unsupported()` не является их общей проверкой.

На producer отсутствуют admission/read/release hooks. Также нет request identity
или picker-session identity для задержанного ответа. Сейчас нет отдельного
popup picker: controls принадлежат уничтожаемому routingPanel Loader. Поэтому
open/close нужно определять через вход/выход из routing settings; нельзя вводить
shadow API и выдавать его за настоящий picker.

**Предложение root до общей production-правки:** добавить только за
`test-support` необязательный observer фактического `Catalog` producer, протянутый
через HostConfig → actor → editor::catalog. События: admitted(request token,
processes), read(class: directory/stat/exe/desktop/header), released(request token).
Token диагностический и не PID selector. Не передавать argv, environ, содержимое
desktop, credential bytes либо host paths в журнал. Observer должен поддерживать
управляемую паузу перед публикацией результата, чтобы close/reopen не зависели
от скорости машины. Release guard обязан работать при ошибке и panic task.
Это предложение, не реализованный API и не согласие на shared production edits.

Не предлагается native permit/N5/fake capability. Если delayed result после
закрытия меняет новый picker, это отдельное наблюдаемое finding; исправление
request/session cancellation требует отдельного root-контракта. Одного добавления
счётчиков недостаточно для объявления такого поведения исправленным.

## Оставшийся эксперимент после согласования

Нужна source-bound GUI fixture с production Main/qrc/bridge/backend/catalog;
P03/P07 original-qrc interposer совместим как pattern, старый ELF — нет.
Сборка: 1.85.1, wrappers пустые, jobs1, slot, directory flock, свой transient;
сохранить ELF/source/log hashes вне TTL до cargo clean.

Controlled own same-UID PID namespace и private Xvfb/software; минимальный runtime
closure копировать из W4 readonly-runtime с проверкой runtime-inputs.json. Текущий
срез runtime closure не копировал и host session не использовал.

Через видимые enabled controls создать query/load/reload и выбрать только
собственный helper. Own .desktop entries должны содержать benign Exec marker,
wrapper/complex/unsupported cases и oversized entry для incomplete. Syscall oracle
должен отличать directory/stat/exe/UID от запрещённых cmdline/environ/credentials;
не использовать настоящие секреты. Положительный canary чтения и отрицательный
контроль запуска marker нужны в том же процессе наблюдения.

Сравнить policy bytes/revision до/после загрузки, выбора, закрытия и reopen.
Приостановить реальный producer, закрыть Loader, затем release; проверить stale
callback. В hidden phase использовать один deadline без polling и screenshots,
проверить нулевую дельту admission/read; затем отдельный visible canary обязан
показать ненулевые счётчики. Отдельно исполнить unknown-kind parser refusals,
known-unsupported Linux mapping и неизменность условий. Пока эти действия не
выполнены, ни один из runtime acceptance пунктов A-08 не считается пройденным.
