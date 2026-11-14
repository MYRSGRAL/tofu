# P-03: интегрированное дополнение к frozen component 0bcf023

## Зафиксированный до campaign scope

Исходные component 3×100/canaries не повторяются неизменно и не переатрибутируются.
Новый example `p03_integrated` включает production bridge/host/qrc с единственным
private dispatch trace/fault wrapper. Перед запуском runner восстанавливает исходный
метод dispatch и требует побайтового совпадения private bridge с production вне него
(нормализуется только relative include bridge_pending). Новый ELF и C++ observer
получают собственные SHA256/source archive/raw bindings. Frozen artifacts 0bcf023
не редактируются. Public contracts, shared W3 application/build/observer не меняются.

После focused oracle preflight предусмотрены **три новых интегрированных 100-cycle
прогона**, поскольку только новый ELF содержит backend dispatch admission и existing
schedule_observer. Новые assertions для смешанной политики и action→model→frame
меняют измеряемую работу. Старые latency/memory значения этому ELF не присваиваются.

Scope — production GUI lifecycle в private user/net/mount/PID/proc/D-Bus namespace,
offscreen/software Qt, installed KDE theme и private tray host; без engine/connect,
TUN, реальных профилей/SecretService, host changes, runtime-16 и чужих UID. Общая
Linux/physical Plasma/G-UI/GPOWER/GPU приёмка остаётся OPEN.

## Непустая политика и семантика представлений

Два правила сохраняются через **production edit_routing** с актуальной ревизией и
положительным routing_ack. Первое смешивает две executable paths, exact/suffix domain
и два CIDR (AND групп, OR внутри); второе — domain-only Direct. Первый action Block,
default Proxy. Проверяются реальный snapshot после backend ack и actual routingEdit1
control, заполняющий поля executable/domain/CIDR. В каждом режиме поля обязаны иметь
одинаковое содержимое, включая скрытые группы. Advanced показывает три группы,
Simple — executable group; скрытые условия не удаляются. На каждом переходе policy,
routing_revision и revision должны совпадать с post-seed baseline. Сохранение/применение
к сети в циклах не вызывается; это проверка сохранения семантики представления
канонической policy, а не подтверждение сетевой атрибуции synthetic executables.

## Backend за пределами QObject дерева

Admission trace записывается **после успешного DesktopHost.dispatch**, не вместо него.
subscription_visibility в production попадает в Control.subscription_visible watch
sender (backend.rs), worker имеет один receiver, созданный перед циклом actor; ветка
changed обновляет scheduled.visible. Существующий read-only with_schedule_observer
вызывается после обработки scheduler policy и load. Он независимо наблюдает actual
effective ui_visible/loaded, а не desired Qt visibility.

Oracle требует начальный visible gate, ровно одну admitted visibility-команду на
каждый close/reopen и соответствующие backend gate transitions. Close считается
завершённым только после actual hidden state и consumed effective false; reopen —
после actual show server ack и effective true. Trace читается инкрементально, без
повторного сканирования накопленного лога. Сырые callback events не скрываются и не
объявляются числом subscriptions: actor loop может наблюдать policy несколько раз.

Source proof: watch receiver принадлежит одному host actor, не создаётся на каждом
окне; initialize имеет host.is_some guard. Dynamic proof: точное число admitted
команд, effective gate transitions, отсутствие увеличения observed editor tree,
FD/threads и проверка памяти. Состояние fields/mode проверяет ровно один mode
notify на действие; visibility admission ловит дублирование send handlers за
пределами editor QObject дерева. При hide освобождается **видимый scheduler gate**;
постоянный receiver tray-host не обязан уничтожаться. Не вводится выдуманный
глобальный счётчик subscribers=0. Таким образом проверяется фактический lifecycle-контракт текущей реализации:
release видимого gate при постоянном bounded host receiver; нет повторного
создания receiver или обработчиков на каждом окне. Это не утверждение о числе
всех внутренних подписок всего приложения.

## Завершение действия и frames

Таймер начинается до actual control/command dispatch. Loader/model readiness требует
settingsPanel.status=Ready и actual rule controls для обеих сохранённых строк. Mode
readiness — фактическое advanced state. Reopen readiness — actual window visible
после production socket show. Readiness наблюдается сразу после dispatch либо в GUI
thread afterAnimating до синхронизации frame; frameSwapped timestamp должен быть
не раньше readiness. Сохраняются отдельно dispatch→ready и dispatch→frame.
Backend gate completion наблюдается отдельно и не подменяется rendered frame.

Перед hidden проверками visible frame canary явно запрашивает один frame и требует
его actual delivery. Во время измеряемого hidden quiet interval нет update/requestUpdate;
после transition settle проверяются frame delta=0 и effective hidden gate. Нули при
отключённом frame observer не могут пройти preflight. Первые 10 циклов warmup;
p95 nearest-rank на остальных 90 отдельно для каждого повтора. Raw PSS/private dirty,
FD/threads включают стоимость observer; baseline не вычитается.

## Focused oracle preflight

Три positive cycles плюс новые faults: отключённый frame observer; drop actual hidden
visibility command; отключённый backend gate observer; второй actual visibility
admission; повреждение текста domain в реальном поле после переключения режима.
Все faults должны упасть в соответствующем oracle до PASS. Старые disconnected-mode/
retained-editor controls component campaign не повторяются неизменно.


## Уточнение model oracle до финального source binding

Первая integrated серия 3×100 сохранена как промежуточная `campaign/`, не как
финальный результат. При проверке oracle выяснено, что visual Repeater delegates
могут принадлежать другому QObject owner. Финальный observer дополнительно держит
QPointer на actual routingEdit1/2 controls и требует их уничтожения вместе с
редактором. Отдельный focused model-preflight: 3 positive cycles и один fault,
который реально detach/reparent сохраняет routingEdit1; fault обязан упасть в
editor tree release. Предыдущие пять faults неизменно не повторяются: их source
binding — observer-v1.cpp; model дополнение имеет собственный source binding.
Финальные три 100-cycle повтора выполняются с новым C++ observer и тем же unchanged
integrated Rust ELF. Component ELF и frozen artifacts 0bcf023 остаются отдельными.

## Финальный результат

**P-03 controlled production lifecycle PASS** в оговорённом offscreen/software
fixture; три remaining gaps component отчёта закрыты новым integrated proof.
Full Linux/physical Plasma/G-UI/GPOWER/GPU остаются **OPEN**.

Финальные три процесса выполнили по 100 циклов, первые 10 каждого — warmup.
Во всех 300 циклах: mixed policy и обе ревизии неизменны; actual Loader/rule rows
готовы; по одному mode notify; после выхода уничтожены 378 наблюдаемых объектов,
включая оба actual rule controls; backend принимает ровно один hide/show command
и переключает effective gate false/true. Всего 201 visibility admission на процесс
(начальный true + 100 пар), 2 seed edits и shutdown, итого 204 successful admissions.
205 loaded schedule observations на процесс, после seed — ровно два на цикл;
reopen добавляет ровно одно observation к close. Нарастания delivery/handlers в
этих фактических command/gate путях не наблюдается, что подтверждено duplicate
admission fault. Постоянный host receiver не выдаётся за освобождаемый QObject.

p95, мс (90 measured samples на действие на процесс):

| Повтор | Rules ready / frame | Advanced ready / frame | Simple ready / frame | Reopen ready / frame | Close→backend hidden gate |
| --- | --- | --- | --- | --- | --- |
| 0 | 13.888 / 17.177 | 0.213 / 2.894 | 0.443 / 2.343 | 1.151 / 1.913 | 2.000 |
| 1 | 12.709 / 15.487 | 0.172 / 2.429 | 0.405 / 1.943 | 1.249 / 2.101 | 1.893 |
| 2 | 12.392 / 15.415 | 0.218 / 2.804 | 0.400 / 2.417 | 1.362 / 2.405 | 1.918 |

Все measured action→frame p95 ниже начального бюджета 100 ms. Закрытое окно не
должно рисовать frame: его completion — actual hidden + consumed backend gate,
а не искусственное ожидание кадра. Во всех 300 hidden intervals счётчик frames
совпадает между closed_gate и hidden_quiet (>=90 ms с transition settle; внутри
не менее 60 ms passive). Ранее прошёл positive visible frame canary; отключённый
frame observer не проходит preflight.

Raw per-cycle метрики, source hashes, policy/fields, gate traces и controls:
[p03-integrated-evidence.json](p03-integrated-evidence.json).
Повторяемое извлечение/проверка без перезапуска workload:
`python3 tools/p03_integrated_report.py local/validation/p03-integrated OUTPUT.json`.

## Память и ресурсы без вычитаний

В final settled hidden samples FD=29 во всех 300 циклах. Reopen FD=29–30 включает
ещё завершающийся actual local show socket; в следующем hidden sample снова 29.
Threads ограничены 9–11, terminal samples всех трёх процессов — 9. Это диапазон
наблюдаемого worker lifecycle, не глобальный threads=0 claim.

Поздние private dirty ranges (циклы 70–99, KiB): 37556–38644, 37592–38288,
37524–38252. Они перекрываются с предыдущими окнами; верхние значения не образуют
устойчивого роста с номером цикла. Final Pss_File практически постоянен внутри
повтора; Pss_Anon/private dirty колеблются в перекрывающемся рабочем диапазоне.
Промежуточный observer-v1 campaign имел скачки PSS на несколько MiB. Разбор уже
сохранённых smaps_rollup локализовал их в Pss_File (например 32354–41072 KiB),
при перекрывающемся Pss_Anon/private dirty. Причина изменения file-backed
proportional share внешне не установлена; изменение числа sharers — возможное
объяснение, не доказанный факт. Никакой поправки/вычитания не применено. Свой
накопительный рост объектов/FD/gates или private dirty, требующий product fix,
на этих трёх финальных 100-cycle повторах не воспроизведён. Долгий uptime за
пределами P-03 этим результатом не закрывается.

## Checks, bindings и сохранность

Rust 1.85.1 build и focused Clippy `--example p03_integrated --features gui,test-support
-- -D warnings` — PASS (не all-targets). Qt6 moc + clang++ — PASS. Ruff 0.17.4
check/format обоих новых Python tools, rustfmt нового example — PASS. Private bridge
вне dispatch сохраняет исходные production bytes (включая исходный стиль);
dispatch добавляет только trace/fault hooks, обычный positive path вызывает
actual host.dispatch один раз. Fault modes никогда не включаются в positive campaign.
GCC16 warnings в Qt headers сохранены в raw build log.

Исходные frozen component артефакты 0bcf023 не менялись. Все новые raw logs,
observer-v1.cpp/runner-v1.py с исходными SHA256, .so/moc, final ELF и exact source
archive сохранены вне TTL в `local/validation/p03-integrated/`. Checked-in
[p03-integrated-bindings.json](p03-integrated-bindings.json) связывает эти данные.
Cargo.lock менял только порядок entries; accepted bytes возвращены после
семантической проверки, зависимости не менялись. После сохранения evidence и
проверки отсутствия собственных exe/cwd/fd references штатный cargo clean удалил
own transient build. Чужие refs/untracked cross-UID файлы не изменялись/не исполнялись.
