# FR16: LocalSocks native technical details

Scope: production LocalHost/LocalSocks, **не full FR16 PASS**. Worker exec/control/
FD-handoff, helpers, SystemTUN, native ABI/pins, backend HostConfig/Actor/DesktopFetch
не изменены. База feature execution: `6dd2a9371357edf1e8947e279018aee48ec72440`
плюс source overlay ниже. Root corrections cec78bb/55c9008 интегрируются отдельно;
старый ELF не называется ELF merged source.

## Реальное поведение

`CompiledConfig::with_local_runtime_logging` вызывается только LocalHost
`CheckedConfig::validate` перед serialization/config-check. Xray — INFO + access:none,
sing — INFO + timestamp:false (stderr имеет actual elapsed prefix). Общий compile(),
SystemTUN и worker log policy сохранены. Нет debug/trace/verbose/periodic stats.

Только runtime spawn LocalHost получает stdout/stderr pipes; check/version — null
(кроме прежней bounded version validation). Два readers принадлежат LocalSession,
создаются сразу после spawn **до write_config**, используют отдельные framers и
immutable EventLog root/session/generation clone. Tokio bytes readiness, максимум
16KiB/32 frames до yield, try-only record. Unknown/oversize/partial EOF — только
saturating rejected counter; ни sample/raw payload, ни hash, Debug/serde не появляются.
При отсутствии подключённого log pipes всё равно bounded drain/discard.

Stop/failure берёт readers, abort+join закрывает pipes без ожидания EOF/disk;
Drop aborts readers. Существующий child kill/reap/cleanup contract сохранён.
Typed codes 7–10 + Option<i32> centiseconds. Перед записью и проекцией проверяются
source=Engine, value grammar; запись требует ненулевой immutable scope.
Disk recovery проверяет ту же проекцию. Version1, 9 numeric fields, 192-byte cap,
ring2000/page256/queue128 и 5×4MiB не изменены. Old code allowlist 1–6 отвергает новые
IDs — это проверено unit-предикатом, **не запуском old binary**; backwards reader
compatibility не обещается. Миграция формата не вводилась.

`LogEvent::technical_details()` — единый renderer исходного English body для
Desktop Row и export/copy/save. Native payload никогда не входит в serde: DTO
содержит только созданную renderer строку. QML product labels RU/EN живут отдельно,
body PlainText, логика по enum/code, не по переводу. LF — только trusted renderer
framing, input control bytes не экспортируются. Поддерживаются только точные source
ANSI prefixes, не общий ANSI-strip. Filters/partial/cursors/counters/явный refresh
и hidden-no-autopoll не ослаблены. UI объясняет withholding unknown/partial input.

## Проверки и пределы

| Gate | Результат |
| --- | --- |
| Rust 1.85.1 lib native-filter | 56 passed / 7 ignored; ignored native/network gates не запускались. Включает classifier, codec и fail-start cleanup. |
| Local log policy | 1 passed: INFO opt-in, default compiler logging disabled. |
| Focused readers | 3 passed: independent streams, partial EOF, flood cancellation, scope7 не переименован в8; full queue128 со stalled disk consumer, Stop deadline250ms и written=0 до release. |
| Desktop logs lib без gui | 5 passed: shared renderer, invalid numeric projection, safe-save/privacy. |
| Clippy | 1.85.1, linux/engine/desktop all-targets, gui+test-support, `-D warnings`, PASS. |
| Actual GUI | Оба pinned cores normal LocalSocks Start → native startup → Page/UI RU/EN/export/copy/save → normal Stop, PASS. Owned HTTPS/peer только потому, что normal host требует health check; дополнительных probes/flows нет. |
| Producer mutation | Подавление только typed native record → scope/native test RED; byte-exact restore → GREEN. |
| Evidence oracle mutations | Actual saved page без native rows и без technical body в export отвергнуты для обоих cores. Это oracle mutations, не выданные за отдельные actual mutant executions. |
| Privacy/control | Classifier и reader canaries/controls/unknown/version/oversize/partial tests GREEN; actual export и numeric disk проверены на synthetic UUID/URI/control canaries. Никакой проверки реальными credentials. |

Actual export до Stop содержит для Xray `NativeXrayStarted` +
`Xray 26.10.31 started`, для sing `NativeSingStarted` + `sing-box started (0.00s)`
(в этом конкретном execution). Оба — session1/generation1; actual disk после Stop
содержит по 6 numeric records и не хранит body. Unknown native output withholding
наблюдался: rejected=8 Xray / 7 sing. Это не универсальная очистка произвольных логов.

Были сохранены два harness RED: metadata-only smoke trace не содержит logs DTO;
/tmp ancestor штатно отвергается disk policy. Исправлен только harness: проверка
production safe-saved page export и private work под безопасными owned ancestors.
Также первый GUI-feature lib-test link требовал binary bridge symbols: lib tests
выполнены без gui, GUI проверен отдельным настоящим binary. ABI/deps не менялись.

## Source/artifact binding

Evidence вне TTL:
`/home/myrs/projects/yonik/local/evidence/fr16-local-c111`.
`execution-binding.json` фиксирует actual ELF SHA, выбранные pinned binary SHA,
базу/source hashes и executed driver snapshot. `feature-source-overlay.tar` вместе
с base commit восстанавливает compiled feature source. GUI ELF сохранён как
`yonik-desktop-feature`. После выполнения driver изменён только ruff wrapping и
удалением unused local binding; exact reviewed delta сохранён отдельно.
Root corrections после build — отдельный reviewed comment/test-name/doc delta,
а не новое native execution. Результаты прежнего P11/FR16 evidence не переписаны.

`local-socks-evidence.json` связывает source/artifacts/results/checks. Screenshots
actual UI сохранены в runtime/{xray,sing-box}/native-{ru,en}.png; visual inspection
English sing показывает отдельные localized product label и English technical body.

Это закрывает только новый LocalSocks technical-details scope. Worker transport,
полная FR16/RUEN/export/recovery приёмка и последующая integrated-head review остаются
отдельными gate. Reader cancellation проверена bounded Tokio tasks, не аварийным
kernel I/O или чужими UID. Host/kernel/settings/keyring не менялись.
