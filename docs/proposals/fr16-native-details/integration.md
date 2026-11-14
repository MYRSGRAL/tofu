# FR16: private classifier GREEN, следующий integration scope

После Root review версии `26db091`. Реализован только
[native_details.rs](../../../crates/yonik-linux/src/event_log/native_details.rs)
и private `mod native_details` в EventLog; точечный allow(dead_code) нужен до bind.
Нет production INFO enable, capture, новых EventCode, disk/UI изменений.
Этот документ обновляет прежние ограничения runtime-grammar: normal INFO теперь
одобрен Root как направление, но включение требует конкретного integration review.

## Реальные normal startup grammar

Sing-box `box.go:507`: успешный Box.Start → Info без tag/context ID,
`sing-box started (` + `F.Seconds(time.Since(createdAt).Seconds())` + `s)`.
Pinned dependency `sing@v0.9.7-0.20261023115459-5dc452e537bf/common/format/fmt.go`
делает `int(seconds*100)`, затем целую часть и ровно две цифры. Provenance, ZIP/member
SHA и фрагмент: [runtime-source.json](runtime-source.json). `createdAt` создаётся из
времени, не config. Только поэтому duration допустим; произвольное числовое поле
native сообщения секретобезопасным не считается.

Classifier допускает `0.00`–`86400.00` (24 часа), без leading zeros целой части,
знака/exponent/NaN/дополнительных цифр, хранит integer centiseconds. Для всех
допущенных значений renderer даёт **точную исходную body**, включая две цифры;
colored/timestamp prefix — framing и в body не входит. Ограничение 24 часа —
privacy/resource policy с withholding за пределами, не обещание всех возможных Go duration.
Framing proposed stderr INFO capture (`log.New`, timestamp=false): `INFO[ssss] `
либо точный source ANSI cyan INFO/reset + `[ssss] `; ssss=4–10 ASCII digits.
`DisableTimestamp: !logOptions.Timestamp && logFilePath != ""` при stderr без
файла равно false. Файловый output с timestamp=false использует только бесцветную форму `INFO `:
`DisableColors: logOptions.DisableColor || logFilePath != ""`. Цветной prefix без
elapsed остаётся допускаемой parser grammar, а не утверждением о file output
через pinned `log.New`. Предлагаемый stderr capture сохраняет elapsed. Не включать общий ANSI-strip и свободный timestamp parser.

Xray `main/run.go:95` вызывает `server.Start()`, `core/xray.go:389–400` после
успешного запуска features выводит **Warning** `Xray 26.10.31 started`. Warning
входит в обычный INFO threshold, дополнительных flow/DNS/verbose не нужно.
`Version()` использует pinned byte constants 26/10/31; никакой config/build string
в body не входит. Prefix: Go calendar timestamp + `[Warning] core: `; context
ordinary constructor не несёт request ID. Parser неизвестные prefixes безопасно
отклоняет, не удаляет их эвристикой. Это **runtime Start**, а не version banner.

Обе реальные startup frames найдены в retained logs; paths/hash/line/exact frames
добавлены в [fixtures.json](fixtures.json). Их чистый replay проходит вместе с двумя
прежними conditional frames (network missing/default local DNS). Historical logs
не присваиваются текущему binary/config. Integrated native availability всё ещё UNPROVEN.

## Точный минимальный production diff для назначения владельцев

1. **Compiler owner:** в `yonik-engine/src/lib.rs::compile` заменить только
   `{"loglevel":"none"}` на `{"loglevel":"info","access":"none"}` для Xray;
   для sing `{"disabled":true}` на
   `{"level":"info","disabled":false,"timestamp":false}`.
   В pinned `option/options.go` DisableColor имеет `json:"-"`: JSON switch для
   отключения цвета нет; classifier принимает только точный известный color prefix. Не менять native user log output на произвольный путь,
   не включать debug/trace/verbose или периодические отчёты. Imported configs — отдельный
   scope: произвольные log options не расширяют grammar. Существующие compile snapshots
   и config-check tests обновляет тот же владелец.
2. **EventLog owner:** добавить 4 typed EventCode, предложенные numeric IDs 7–10:
   NativeSingStarted, NativeSingMissingInterface, NativeXrayStarted, NativeXrayLocalDns.
   Существующее `LogEvent.value` — **Option<i32>**, отдельного EventValue enum нет.
   Для ID7 — Some(centiseconds), остальных None. Detail закрыт, no Debug/Serialize;
   production record adapter потребляет Detail и mapping enum, не принимает native
   String/сырой i32 снаружи. Перед UI/export projection повторно валидировать код/value
   (duration range и обязательность значения), неизвестные комбинации withheld.
3. **Disk owner:** существующая запись version1 уже имеет numeric code + presence/value
   i32 + root-bound session/generation. Добавить четыре match arm в `disk::encode` и
   разрешённые IDs в `valid`, без смены 9-field layout/version и без migration.
   Record192 остаётся достаточным. Старый reader откажет на неизвестном ID — не
   обещать backwards reader compatibility; это отдельный compatibility тест.
   Body не писать в disk. Новые typed template данные не получают отдельный raw store.
4. **Local host reader owner:** `host/process.rs::spawn(checking=false)` получает
   stdout/stderr pipes; Check/version paths остаются вне runtime capture.
   `host/session.rs` создаёт reader owner **до** write_config/start completion:
   по Framer на stream, bound выбранный Engine/version из existing selection,
   immutable EventLog root/session/generation. Reader callback делает только try-record.
   Привязка generation из sink, а не текущего UI snapshot. На bytes readiness читать
   до 16KiB/32 frames за turn, затем yield; никакого timer/poll/idle loop. При busy/full
   продолжать bounded drain/discard и saturating loss counters, не ждать EventLog/disk.
   Stop отменяет/закрывает reader и scratch; EOF partial не публикуется. Не ждать
   stream EOF/consumer flush в Stop ACK. Проверить pipe flood fairness/Stop интеграционно.
5. **Worker path — отдельный транспортный scope до реализации:** worker Run делает
   `command.exec()` (`worker.rs:818+`), перед этим закрывает bootstrap UnixStream.
   В нём нет parent reader task; нельзя просто написать `.stdout(piped())` и объявить
   работоспособность. Минимальное направление без exec monitor — передать dedicated
   output pipe FDs существующим FD-handoff, привязать reader в session owner, затем
   worker задаёт `Stdio::from(fd)` и exec сохраняет accepted child identity/lifecycle.
   Нужны явные descriptor role/count/ownership/close-on-failure правила у existing
   payload/transfer endpoints; это требует отдельного согласованного wire scope и
   владельца. Не смешивать raw native с YNKA/control/ACK stream и не использовать
   journald как raw fallback. Здесь helpers/wire не изменены. Local-host proof нельзя
   объявлять worker-integrated proof.
6. **Desktop/W3A08 owner:** typed Row carries template enum + validated duration;
   RU/EN product codes из каталога, native body отдельно PlainText. Export renderer
   использует ту же projection, LF только trusted framing. Нет native bytes, paths,
   URL, secret samples/hash ни в UI, clipboard, disk, Debug. Сохранить page partial,
   source/code filters, evicted/rejected/missed/disk loss. Не сравнивать переводы.

Пункты 1/2/4 можно поручить после scoped review classifier; worker descriptor transport
нельзя молча включить в «малую pipe правку». Ни один пункт production здесь не выполнен.

## Pure evidence и следующий runtime gate

9 тестов: обе normal startup, прежние runtime constants, duration roundtrip/bounds,
calendar/prefix, canary prefix/suffix/body/control/invalid UTF-8, unknown version,
fragment framing, oversize discard-through-LF/resync, loss/saturating counters,
turn byte/line budget и Stop (partial erased, feed после Stop не работает).
Loss моделируется try-offer=false; реальная queue128/OS cancellation здесь не тестировалась.
Scope root/generation — обязанность integration sink, parser не создаёт authority.

Rust 1.85.1 standalone test harness включает ровно private module: rustc --test
-D warnings, затем test binary, затем clippy-driver --test -D warnings. Всё под
existing `local/orchestration/with-rust-slot` и shared flock task-root; jobs=1.
Cargo manifests/deps не изменялись, dependency/workspace build не выполнялся.
[RED](checks/red.log), [GREEN](checks/green.log); clippy.log пуст при успехе.
Это focused classifier build/test/clippy, не whole-crate cargo all-targets verdict.

Следующий gate после конкретного scope/review: включить нормальный INFO только
в согласованном compiler path, захватить Start обоих выбранных pinned cores через
same-UID loopback fixture без connect/probe/DNS/idle traffic, затем Stop. Требовать
actual producer→EventLog→UI/export witness и loss/control/Stop integration negatives.
Xray ordinary Start Warning и sing ordinary Start Info теперь source-bound достижимы,
но execution здесь не разрешён и не выполнялся. Full FR16 PASS по pure parser запрещён.

## Узкая поправка provenance после R2

Исправлены атрибуция prefix, комментарий и имя/expect теста файлового output.
Parser и assertions не изменены. GREEN/clippy в checks относятся к 46bbd284;
новый запуск или сборка этим исправлением не заявляются.
