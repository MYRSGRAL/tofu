# FR16: доступность безопасных исходных технических сообщений

Статус: предложение для Root, **не FR16 PASS**, без producer implementation.
База: `61cfa0f69134bcfe286a44506b5191f4bd3c9d9e`; только новые документы/fixtures.
Применение контракта к production требует отдельного согласования Root.

## Норматив и точный остаток

[FR-16](../../specs/requirements.md#L30): разделение App/Engine, фильтрация,
ограничение размера, безопасный экспорт. [Validation](../../specs/validation-and-roadmap.md#L192)
добавляет rotation/backpressure и редактирование секретов/управляющих последовательностей.
[data-and-contracts:93](../../specs/data-and-contracts.md#L93) требует доступности
исходных сообщений стороннего ядра как технических деталей; не требует сохранения
произвольных недоверенных bytes и не разрешает исключение из privacy.

Принятые #189 disk/host/CLEAN и merged #302 не переаудируются.
[Ledger #192 / delta](../../validation/2026-11-14-linux-delta-302f8bf.md#L44)
оставляет native availability UNPROVEN. Этот документ подтверждает конкретный
source gap, но не повышает статус runtime acceptance.

| Путь на базе | Producer → presentation/export | Вывод |
| --- | --- | --- |
| Рабочий core/check | [worker.rs:665](../../../crates/yonik-linux/src/worker.rs#L665) `command`: stdin/stdout/stderr null; configured_assets заменяет только stdin конфигом | stdout/stderr не поступают в EventLog вообще |
| Проверка версии | [worker.rs:710](../../../crates/yonik-linux/src/worker.rs#L710): отдельный `version`, stdout pipe, take(4097), лимит 4096, проверка prefix; stderr null | Локальные bytes для проверки, не technical-details API; suffix-prefix проверка не является safe parser для журнала |
| Произвольные bytes | [event_log.rs:168](../../../crates/yonik-linux/src/event_log.rs#L168) `reject_native` → reject counter; вызовы в тестах | Нет production stdout→reject_native; нет хранения/публикации текста |
| Wrapper lifecycle | [session.rs:234](../../../crates/yonik-linux/src/host/session.rs#L234) ProcessSpawned через App; [process.rs:142](../../../crates/yonik-linux/src/host/process.rs#L142) Engine ProcessExited после reap OS status | Это wrapper-события, не оригинальный текст ядра |
| EventLog → Desktop | [logs.rs:34](../../../crates/yonik-desktop/src/logs.rs#L34) read_scoped → Row: id/source/code/value/context; scoped.export → page partial | Только закрытые коды/числа |
| UI → copy/save | [LogsView.qml:75](../../../crates/yonik-desktop/qml/LogsView.qml#L75), [logs export](../../../crates/yonik-desktop/qml/LogsView.qml#L109): PlainText, явная страница, copy/save | Нет поля исходного native message |

Поиск иных путей technical/details/native message в desktop/linux production
не обнаружил альтернативного пользовательского канала. Broker/worker typed protocol,
version validation и test stdout не подменяют такой канал. Нормативная доступность
исходных сообщений сейчас не покрыта. Безопасность текущего closed-code export
не доказывает безопасность ещё не существующего native parser.

## Предлагаемый частный API v1

Первый этап — только известные константные сообщения версии обоих pinned cores.
[fixtures.json](fixtures.json) содержит точные retained строки, source-lock commits,
хэши файлов и происхождение. Xray строка содержит конкретный build/toolchain:
другая сборка даже той же версии требует нового template ID и evidence.
Это реальные retained build/version stdout, **не runtime-run log witness**.
Их публикация решит доступность этих сообщений, но не автоматически всего
норматива «исходные сообщения»: нужен Root mapping представительного runtime
сообщения каждого ядра либо явное решение о достаточности version-only scope.
На текущем evidence безопасная runtime grammar обоих ядер остаётся неизвестной.

Предлагаемые сигнатуры (эскиз, не новый public transport):

```rust
// Типы не реализуют Debug/Serialize; поля закрыты.
fn bind_native_details(root: &EventLog, scope: AppliedScope,
    engine: Engine, phase: NativePhase) -> NativeDetailsSink;
fn classify_line(sink: &NativeDetailsSink, stream: NativeStream,
    bounded_line: &[u8]) -> Result<SafeNativeDetail, WithheldReason>;
fn try_record_native(sink: &NativeDetailsSink, detail: SafeNativeDetail) -> RecordOutcome;
```

`AppliedScope` фиксирует root identity журнала, session_id и generation в момент
создания child/version-check. Не брать generation из текущего UI snapshot и не
перетегировать поздний output. Scope/identity строит доверенный владелец worker;
core bytes не могут задавать root, version, ID или контекст. Version-check до
сессии нельзя приписывать активной сессии: отдельная явная область verification
либо отказ до появления допустимого контекста. Для runtime phase version-only
правило не разрешено: такая строка сама по себе не доказывает runtime событие.

Исправление после Root review: **новый VerifiedPinnedIdentity authority gate не нужен**.
В сигнатуре используется существующий выбранный
`engine: Engine`; это ссылка на уже принятый pinned-core TCB и существующий binary
selection/provenance binding, не создание нового источника authority. stdout никогда
не удостоверяет identity. Version mismatch withholding относится к известной версии
выбранного core, а не к самоавторизации входной строкой.

`SafeNativeDetail`: schema=1, engine(enum), template_id(enum), phase(enum),
stream(stdout/stderr enum), session/generation из sink. Никаких свободных String,
raw bytes, config/path/URL, native payload hash или secret correlation ID.
Version выводится из template, а не из произвольного поля. На первом этапе
переменных native полей **нет**. Будущие поля требуют source-bound closed grammar,
доказательства отсутствия секрета по происхождению и жёстких enum/range bounds;
одной проверки «число/ASCII» недостаточно, число тоже может кодировать секрет.

Парсер сравнивает всю строку с точным разрешённым template, исключая только
один завершающий LF framing. Суффикс, CRLF, ANSI, NUL, invalid UTF-8, bidi,
подмена цифр, несколько строк, неизвестные сообщения → withheld. Никакого
whole-line redaction по regex, ANSI-strip или эвристики «не похоже на пароль»:
они не гарантируют удаления секрета. Не публиковать фрагмент отказанной строки.

## Bounds, lifecycle и хранение

Предлагаемые read bounds: максимум 4096 bytes строки, фиксированный scratch buffer,
переполнение → discard до LF без накопления; один oversize counter на строку.
За один scheduled turn не более 16 KiB/32 строк, затем yield; stdout/stderr имеют
отдельный framing buffer. Счётчики saturating, неизвестные/encoding/oversize/scope
различимы фиксированными reason codes без sample bytes. Drop bytes не хэшировать.

Reader живёт вместе с конкретным child и неизменным sink. Stop отменяет reader,
закрывает pipes и отбрасывает partial frame; не ждёт EOF/flush/disk consumer.
Ни tasks, ни refs не переживают владельца scope. Нельзя блокировать child заполненной
pipe: при перегрузке draining/discard с ограниченным CPU и loss counters; точная
реализация и тест fairness/Stop обязательны до включения. Запрещены forced verbose,
новые probes/version executions ради журнала, idle polling/activity.

Сохранить ring 2000, page 256, очередь 128, disk 5×4MiB, record ≤192 bytes,
существующие evicted/rejected/missed/disk-drop counters и nonblocking producer/Stop.
Не добавлять unbounded side store. Компактный disk record хранит numeric schema,
engine/template/phase/stream и scope, не текст; renderer восстанавливает точную
одобренную исходную английскую строку из immutable versioned table. Ограничение
192 включает все поля и framing; неизвестная schema при recovery не рендерится.
Это предложение расширения формата, совместимость/recovery потребуют отдельного
согласованного решения и тестов, а не молчаливой смены текущего parser.

## UI/export и ownership следующего этапа

Продуктовые codes должны иметь RU/EN presentation в каталоге переводов;
исходная английская строка показывается отдельными «Техническими деталями» с
engine/version/phase и отметкой partial/withheld, не как переведённый UI.
Логика работает по enum/template/status, никогда по локализованной строке.
Сейчас UI показывает `modelData.code`; это не новый finding для исправления здесь:
product localization принадлежит W3A08. Согласовать с его владельцем новый Row.

Технические детали только Text.PlainText / TextEdit.PlainText; web consumers
через textContent. Не HTML/Markdown и не auto-link URL. Экспорт — явная ограниченная
страница, UTF-8 plain text; допускаются только доверенные LF разделители renderer,
никаких ESC/NUL/CR/C0/C1/bidi из input. Сохранение через существующий safe-save;
никаких config/path, raw payload или новых Debug/serde обходов. UI/clipboard/file
получают одинаковую разрешённую проекцию; withheld отражает счётчик и объяснение,
никогда исходный input. Это не гарантия очистки произвольных чужих файлов.

Root может назначить минимальный implementation scope: новый private
`yonik-linux/src/event_log/native_details.rs` (классификация/закрытые типы), затем
отдельно согласованные worker capture/bind points, EventLog/disk typed projection,
Desktop Row и QML details presentation. Не менять engine/Go/helpers/Cargo/deps.
В этом PR эти файлы **не изменяются**. Disk migration и W3A08 integration требуют
владельцев до shared edits; W4VM/P02 файлы не затронуты.

## Проверяемая матрица и следующий gate

JSON fixtures — source-bound данные/ожидаемые результаты, **не исполненный parser**.
Positive: обе точные version строки с provenance. Negative: secret prefix/suffix/
field, controls/invalid UTF-8, markup, unknown version/grammar, oversize+resync.
Lifecycle/bounds: queue-full, ring eviction, stale generation, wrong root,
unterminated Stop, page overflow. Будущие pure tests обязаны проверить fixtures
против реализации, включая отсутствие строки canary во всех UI/export/disk outputs,
раздельные counters, размер record и отсутствие late-generation relabel.
Drop tests проверяют отсутствие ожидания; producer/Stop integration нужны отдельно.

До FR16 verdict нужны source-bound runtime witness для обоих pinned cores,
разрешённый Root grammar/scope, положительная публикация и безопасные отказы
через настоящий producer→UI/export. Одних fixtures, version banner или wrapper
exit недостаточно. Rotation/host/P07 повторно не проверяются этим предложением.

## Уточнение после Root review

Version-only недостаточно. Следующий bounded результат — [runtime grammar](runtime-grammar.md)
и [source excerpts](runtime-source.json). Он заменяет version-only направление и
предложение нового identity gate выше; остальные privacy/bounds остаются. Ни capture,
ни disk/UI реализация не разрешены этим документом.
