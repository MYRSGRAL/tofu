# FR16: source-bound runtime grammar после Root review

База исследования `8e8baf537406e7bc5efaf5ccaaf8867de20e2a9b` (PR196).
Только read existing source/logs и docs/fixtures; ничего не запускалось из Go/core,
не скачивалось, не собиралось. Ни capture, ни disk/UI, ни новый authority gate.

## Доказательства и ограничения

Прочитаны retained `upstream.tar` из rehearsal `14229843-msrv85-mtree4`.
SHA-256 **совпадают с source-lock**: sing-box `b17af994…a21472b`, Xray
`7dba1dd0…ec51b53`. Точные SHA, archive/member paths, member SHA и ограниченные
фрагменты с номерами строк: [runtime-source.json](runtime-source.json).
Это pinned upstream source; текущие adapter patches поиском не изменяют выбранные
`route/network.go`, `log/format.go`, `log/log.go`, `app/dns/nameserver_local.go`,
`common/log/logger.go`, `common/errors/errors.go`, `infra/conf/log.go`.
Не делается новый вывод об идентичности запущенного артефакта по его output.

В [fixtures.json](fixtures.json) сохранены две **реальные** строки исторических
native test logs: sing-box `pipeline-e5a27/sing.log:6` и Xray
`tun-e5a27/xray-final-tests.log:237`, с SHA файла и исходным framing.
Это не выдуманное время и не новый запуск. Они доказывают существование конкретных
форм вывода, но не новый integrated worker→EventLog→UI/export witness и не актуальную
идентичность binaries. Их контекст: [pipeline](../../validation/2026-11-12-tun-core-pipeline.md)
и [closed native TUN](../../validation/2026-11-12-closed-native-tun.md).
Данные из соседних строк логов не копируются. Runtime evidence доступно локально
по указанным абсолютным путям; portability обеспечивается двумя tiny fixtures,
а полный лог не добавляется в репозиторий.

## Два допустимых constant-body template

| Template | Исходный body ровно | Источник и условия |
| --- | --- | --- |
| sing-network-missing-v1 | `missing default interface` | `route/network.go:508–514`, dispatchInterfaceUpdateLocked, отсутствует DefaultInterface; pause + Error, без error object/адреса/пути. `box.go:228` задаёт постоянный tag `network`. Это runtime network event, **не нормальный успешный startup**. Не вызывать его изменением host/network. |
| xray-local-dns-created-v1 | `DNS: created localhost client` | `app/dns/nameserver_local.go:42–47`, NewLocalNameServer, Info(context.Background), без account/URL/path/address/port. Это runtime construction DNS component, не доказательство успешного DNS request. |

Значение слова `localhost` во втором body — исходная константа сообщения, не
подставляемый адрес. Оба body не принимают ни одного native переменного поля.
Это source-safe accepted **кандидаты контракта**, условные по достижимости; не
утверждение, что current configs их генерируют. Исходная английская body строка
рендерится ровно из fixed trusted template, отдельно от RU/EN продуктового кода.

Полная frame grammar задана в JSON. Она anchored на весь frame, включая единственный
LF, version берётся из существующего выбора core. Prefix sing: ровно `ERROR`,
либо ровно исходный formatter `ESC[31mERROR ESC[0m` без пробела между ERROR и ESC,
затем `[ssss] network: `, где ssss — 4–10 десятичных цифр elapsed whole seconds.
Число происходит из `timestamp.Sub(BaseTime)/time.Second`, не из config или message.
Большие/отрицательные elapsed значения безопасно withheld. Не путать этот prefix
с переменной startup duration в body `sing-box started (...)` — она **не принята**.

Prefix Xray: `YYYY/MM/DD hh:mm:ss.uuuuuu [Info] app/dns: `, ASCII digits фиксированной
ширины, календарные поля дополнительно валидируются как дата/время (секунды 0–59,
год 1–9999), без свободных suffix, thread/request ID или inner-error chain.
`common/log/logger.go:152` использует Go date/time/microseconds; caller package
получается из runtime.Caller, background context не добавляет request ID.
Timestamp/elapsed — только проверяемое framing, не arbitrary native metadata.

Первый минимальный stored event: engine + template enum + immutable session/generation;
никакого native текста или timestamp в storage. Рендерится **точная исходная body**,
а не исходная decorated line: timestamp/elapsed/color не обещаются сохранёнными.
Это различие должен принять Root при нормативном mapping; для byte-exact whole-line
availability этот вариант недостаточен. Не хранить raw frame ради такого обещания.

Известные ANSI bytes допускаются только в одном точно описанном prefix sing;
любой дополнительный ESC, OSC, CR/NUL/TAB, C1, bidi, invalid UTF-8 и markup вне
констант → withheld. В экспорт не выходит даже разрешённый color prefix. Это
**проверка целой закрытой grammar**, не ANSI-strip плюс эвристика секретов.
Secret prefix/suffix/подмена body всегда отказываются; не допускается substring match.
У неизвестных сообщений нет sample/raw/hash, только saturating reason counter.

## Фактическая достижимость при current config

[compiler](../../../crates/yonik-engine/src/lib.rs#L114) ставит Xray `loglevel=none`
и sing-box `disabled=true`. В pinned source `infra/conf/log.go:57–59` отключает
access/error writers; `log/log.go:25–26` возвращает NOP factory. Поэтому **ни одна
из этих двух grammar не даёт positive при обычном generated runtime config**,
даже после будущей замены null на pipes. Поддержка grammar сама по себе gap не закрывает.
Native imported config может иметь другие настройки, но его свободные log options
не должны автоматически расширять разрешённую grammar.

Xray `main/run.go:82` печатает banner даже при none, однако Root уже отклонил
version-only как достаточное выполнение. `Configuration OK.` — check/test, не
runtime startup. `Using config from STDIN` — условная ветка fallback verbose,
не свидетельство текущего `-c stdin:`. Для sing-box успешный Start пишет Info
`sing-box started (` + elapsed + `s)` (`box.go:507`), не constant-only message.
Не подменять это wrapper ProcessSpawned, не включать trace/debug/verbose ради witness.

## Минимальный producer bind и следующий gate

Использовать существующий выбранный pinned Engine и его accepted provenance.
Никаких VerifiedPinnedIdentity, exec monitor, новой source identity системы.
Приватный sink создаётся владельцем child из уже существующих EventLog root,
session и generation; scope immutable. stdout не может менять Engine или scope.
Условный private classifier принимает bounded frame и known selected Engine,
возвращает только enum template или reason. Capture/bind в worker, disk encoding
и UI projection **отложены**, их реализация здесь не предлагается как выполненная.
Ring/page/queue 2000/256/128, record192, 5×4MiB, counters и nonblocking Stop неизменны.

Минимальный diff следующего этапа после Root review: private classifier + pure
fixture tests сначала; producer capture отдельно после решения о достижимости.
Fixtures дополнены для обоих runtime templates: retained positive, synthetic
secret prefix/suffix, injected controls, unknown selected version, oversized frame,
loss. Positive означает корректность conditional classification, не production emit.
Существующие scope/Stop/page/queue fixtures из proposal сохраняются.

**Runtime gate:** без изменения log policy и без искусственных host events
same-UID loopback не может породить выбранную sing-box константу. Поэтому безопасный
универсальный «один loopback запуск докажет обе grammar» сейчас предложить нельзя.
Нельзя сфабриковать witness или принудительно выключить default interface.
Root должен сначала выбрать допустимый observable runtime scope при запрете
forced verbose/idle: оставить logging disabled означает признать availability
неподтверждённой; разрешить обычный Info для ограниченного controlled witness —
отдельное решение о log policy, **не инструкция исполнять сейчас**.

Точный минимальный план **для review до execution**, если Root разрешит обычный Info
и расширение sing startup grammar безопасным source-derived elapsed полем:
переиспользовать same-UID loopback fixture из существующих compiler/native lifecycle
тестов, существующие выбранные pinned binaries и bound session; один обычный Start,
никаких probes/idle load/TUN/kernel/host changes; localhost-only SOCKS listener,
без remote account/traffic; считать bounded startup frames, затем обычный Stop.
Для sing-box подтвердить нормальный `started` (новая grammar ещё требует отдельного
source/formatter field review); для Xray выбрать и подтвердить достижимый постоянный
runtime body при этом fixture. Local DNS constructor не гарантирован simple SOCKS
config, потому до исполнения проверить compile path, не добавлять DNS activity ради
логов. Этот план **не ready-to-run gate**: он явно требует двух Root решений и
закрытия Xray reachability. Альтернатива сейчас — только pure replay двух retained
frames, с честной отметкой historical/source-only. Новых engine executions не было.

FR16 full PASS по этому документу запрещён: нужны согласованный normative scope,
реально достижимые producers при разрешённой policy и integrated UI/export witness.
