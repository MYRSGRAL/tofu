# P-01: bounded proposal перед длительной кампанией

Статус исходного preflight: **OPEN / API blocker**, не PASS и не результат измерения idle.

Продолжение: root разрешил узкий test-support observer, реализованный в
[private API v1](p01-observer-api.md). Full-duration кампания запущена; этот
документ сохраняет первоначальный proposal, не описывает её итог.
Root подтвердил N/A stats-positive-canary только для frozen 3e64220;
появление production statistics требует новой проверки.
База: accepted `origin/main` `3e64220fc16b7cfdd2170bd0fffc7049d056c8a7`.
Рабочая ветка: `test/p01-idle-acceptance-3e64220`.
Проверка API выполнена по исходникам этой базы; старые GUI ELF не запускались.
Сборка не запускалась: прежде нужен согласованный oracle. Source/ELF binding
и численных результатов пока нет. Kernel receipts и чужие файлы не изменены.

## Доказанное ограничение существующего fixture

- `Main.qml:16`: normal ApplicationWindow создаётся видимым. В CLI `main.rs`
  нет start-closed. Внешнее закрытие после запуска возможно, но не доказывает
  отсутствие начальных кадров; начало измерения после такого закрытия нужно
  явно отличать от настоящего start-closed.
- `main.rs:71–85`: ephemeral-key fixture включается только `--smoke-test`.
  Нормальный GUI использует HostConfig без этого обхода Secret Service.
- `application.cpp:457–463`: любой smoke-test вызывает shutdown через 45 000 мс.
  Это прямое препятствие 900-секундной фазе существующим smoke fixture.
- `application.cpp:381–385`: script дополнительно запускает таймер 25 мс.
  Такая нагрузка загрязняет измерение собственных периодических задач.
- `application.cpp:419–426,446–450`: trace записывается только в smoke-test;
  visibleChanged тоже наблюдается только там. Метаданные не содержат счётчиков
  frameSwapped, обновлений UI-статистики или backend statistic polling.
- `application.cpp:94–117`: существующий captureWindow сам вызывает update()
  и grabWindow(); это активный screenshot helper, а не пассивный redraw oracle.

Точные выдержки с номерами строк и SHA256 источников:
[api-excerpts](../evidence/p01-api-preflight-20261114/api-excerpts.txt),
[context](../evidence/p01-api-preflight-20261114/context.json).
Это API preflight, не runtime-воспроизведение 45-секундного shutdown. Отсутствие
счётчиков не доказывает production bug или наличие скрытых кадров/опросов.

## Предложение root в пределах shared contracts

Нужен согласованный узкий acceptance observer/control для **actual Main.qml,
production backend и bridge**. Эти файлы здесь только прочитаны. Предложение:

1. До первого показа окна разрешить fixture start-closed; управление open/quit
   выполнить через существующие QAction. Разрешить FR07 ephemeral credential
   fixture отдельно от smokeSteps/deadline либо использовать изолированный
   настоящий Secret Service. Ни одного backend connect/probe bootstrap.
2. Пассивно считать frameSwapped, visibleChanged, доставку UI statistics и
   фактическое выполнение statistic polling; сохранить начальное/конечное
   значение на границах фаз без таймера в измеряемом процессе. Не заменять
   stats счётчиком всех snapshot: это другой контракт.
3. Проверить чувствительность oracle отдельным коротким negative canary:
   намеренно запросить кадр в видимой фазе и проверить прирост; для stats
   явно определить наблюдаемое событие. Canary не включать в измеряемые фазы.
   Если root выбирает внешний Qt profiler вместо hook, сначала подтвердить
   доступность этих событий и измерить overhead на том же ELF.

После решения root: одна release GUI сборка на frozen source с manifest всех
исходников, командой и toolchain, SHA256 ELF; сохранить ELF вне TTL. Сборка
через общий max-2-slot wrapper, jobs=1 и BSD flock корня своего transient.
После сохранения ELF и evidence — cargo clean и удаление только своего root.

## Ограниченная будущая кампания (не запускалась)

Три независимых повтора последовательно: 900 секунд hidden/no-connect,
затем 900 секунд static-open; окно не содержит активного поля ввода/анимации.
Внешний bounded script хранит samples и terminal result, общий deadline с
запасом на setup/quit, на ошибке завершает только собственные PID groups.
Границы фаз задаются monotonic clock после подтверждения GUI visibility.
Один внешний sample каждые 5 секунд: CPU ticks всех threads/process group,
PSS/private dirty из smaps_rollup, контекстные переключения threads, process
inventory/argv, load, power/governor/доступная температура. Context switches
публиковать как proxy, не называть точным числом timer wakeups; для wakeups
нужен отдельный доступный scheduler/timer oracle и указание его overhead.
SNI fixture PID считать отдельной строкой и включать в сумму группы;
сборщик и private bus учитывать отдельно как инфраструктуру.

Для каждой фазы: CPU seconds и процент одного CPU, PSS/private dirty
min/median/max, wakeups с указанием метода, delta frames/stats, затем разброс
между тремя повторами. Устойчивый hidden CPU >0,2% — нарушение начального
бюджета, ненулевые скрытые frames/stats — нарушение событийного gate.
Inventory должен подтвердить отсутствие engine/probe-runtime во всех samples;
краткоживущие процессы требуют event witness, одного snapshot недостаточно.
Никаких 16 probes, worker bootstrap, syscall/cross-UID экспериментов.

CPU/PSS/wakeups/redraw: **не измерены**, variance: **нет данных**.
Все runtime gates P-01 остаются OPEN. Private offscreen/software fixture
не закрывает physical Plasma, G-POWER, battery/AC, P-02, P-03 или Android.
Host context из preflight не заменяет context и температурный диапазон каждого
будущего прогона; другие writer builds могут влиять на шум.
