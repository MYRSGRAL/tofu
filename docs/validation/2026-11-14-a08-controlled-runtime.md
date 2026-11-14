# A-08: controlled production GUI/catalog и remaining finding

Продолжение `d325033` на той же task-ветке от accepted main `1461774`.
Предыдущий preflight и его binding сохранены как отдельное историческое evidence.
**A-08 OPEN:** controlled проверки ниже прошли, но reopen отображает catalog
другого режима. Physical Linux — DEFERRED, не PASS. Полный FR-20/G-APP/native
admission не проверялся.

## Реальный путь и граница изменений

Observer включается только явным `HostConfig::with_catalog_observer` с
`test-support`: HostConfig → Actor → editor::catalog → тот же Catalog IO.
Без feature поля HostConfig/Actor/Catalog отсутствуют. Bridge, QML, JSON команды,
production dependencies, N5/helper/nsfs/engine compiler не изменены.
`Io` без feature — пустой контекст. События содержат только monotonic per-instance
token, processes и закрытый класс Dir/Stat/Exe/Desktop/Header/Status. Это счётчики
попыток файловых операций в существующих wrappers, не число kernel read syscalls.
Observer не передаёт paths/argv/environ/raw desktop или credentials.

RAII session живёт внутри настоящего snapshot. Проверки fixture перед GUI
исполняют missing-proc error дважды (tokens 1/2 и оба release) и panic на Read
(catch_unwind; release token 1). Ожидаемый пойманный panic виден в stderr.
Далее отдельный observer экземпляр обслуживает настоящий GUI producer. Только
его one-shot arm-файл в уникальном own fixture удерживает release перед
публикацией до signal-файла; deadline 8 s. Token не участвует в authority,
отмене или product session semantics. При callback ошибки сам observer не
обещает изоляцию panic; release callback fixture должен успешно завершаться.

`examples/a08_gui.rs` использует **исходные** bridge.rs/bridge_impl.rs,
application.cpp и production qrc Main. Interposer читает ресурс и сверяет его
SHA256, затем вызывает clicked handlers существующих видимых enabled controls.
Настоящая fixture v2 policy с Android selector и доменом записана через encrypted
repository и штатный format-3 backup до запуска host. GUI получает её штатным
startup чтением, без подмены snapshot. Qt input properties и clicked handlers
проверены; физические mouse/keyboard события и физическая сессия не измерялись.

Собственный PID/mount/net/user namespace, private Xvfb/software. Host UID 1000
отображён в namespace UID 0; все кандидаты принадлежат одному host UID. Модель
изоляции соответствует P03. Попытка numeric --map-current-user была отклонена
самой mount утилитой и не применена. Host session и physical settings не тронуты.
HOME/XDG/runtime и оба system applications roots заменены своими каталогами
в private mount namespace. Единственный copied runtime executable — Xvfb из W4
readonly-runtime: bytes совпали с usr/bin/Xvfb из уже подписанного package;
package SHA и runtime-inputs manifest SHA сохранены. Install/download не было.

## Пройденные controlled проверки

Final runs: `local/validation/a08-runtime/final-{limited,complete,drop,hidden}`.
Свежие компактные результаты: `docs/evidence/a08/controlled-{limited,complete}.json`
и `negative-controls.json`. Full logs, ptrace records, actual Catalog events,
ELF/source archive и hashes остаются вне TTL в `local/validation/a08-runtime/`.

1. Отдельно проверены unknown kind: Draft parser отказывает, engine read_policy
   возвращает InvalidJson. Known Android input успешно сохраняется и production
   GUI отображает точный `unsupported_selector`, rule_id=8 и объяснение #8.
   Это не generic Unsupported() и не неизвестный selector, принятый как пустой.
2. Production Edit/Confirm сохраняет и platform_sources, и домен example.com.
   Revision растёт ровно на один (1→2). Дальнейшие catalog load, selection,
   close/reopen не меняют policy bytes в JSON и revision=2.
3. Own native .desktop Exec указывает на helper с benign marker argument.
   Helper control отдельно создаёт marker-control; catalog **не** создаёт
   EXEC-MUST-NOT-EXIST. Wrapper row имеет `WrapperOrInterpreter` и null executable;
   исходный QML связывает enabled с наличием executable.
   Oversized .desktop даёт incomplete и видимое точное объяснение. Контроль без
   oversized при тех же доступных roots получает incomplete=false и скрытое
   объяснение. Это причинная проверка лимита, не missing-system-root эффект.
4. Process checkbox + Load проходит настоящий proc producer. Выбран только
   собственный `a08-helper`; selection заполняет executable, не сохраняет PID
   или новое правило. В namespace 7 собственных процессов, никаких чужих UID.
5. Token 3 process scan завершает IO, удерживается перед publication. Back
   уничтожает routingPanel Loader (QPointer=null). В hidden picker interval
   500+500 ms нет polling/screenshots/property reads между deadlines. Новых
   admissions/reads 0/0; допускается **один finishing release** после signal.
   Это не утверждение отмены IO, начавшегося до закрытия, и не измерение uptime.
6. Reopen + Load даёт token 4 и ненулевой visible IO canary. Для каждого
   producer запроса 1…4 существует ровно один release. Политика сохраняется.
7. Независимый ptrace наблюдает только собственный дочерний fixture/thread tree,
   filename syscall arguments, не argv/environ contents. Final GUI trace:
   28 Stat, 34 Exe, 52 other proc file operations; cmdline/environ = 0.
   Один unresolved metadata FD при newfstatat (ENOENT) отмечен отдельно:
   это не content-read syscall, не спрятанная ошибка декодирования.
   Self canary в чистом synthetic env читает cmdline/environ и даёт оба класса;
   содержимое нигде не записывается. Scope — перечисленные filename syscalls
   x86_64 и собственный namespace; foreign UID campaign не проводилась.
8. Negative drop пропускает реальный clicked Load: detector получает deadline.
   Negative hidden-scan отправляет fault-запрос в **существующий** bridge.dispatch
   при закрытом Loader; реальные counters увеличиваются, detector получает
   `hidden no NEW admission`. Это только negative fault, не acceptance action
   и не новый shadow selector API. Оба final negative runs имеют nonzero exit.

## Найденное remaining поведение; отдельный proposal root

Delayed token 3 фильтрует процессы по a08-helper. После release общий
`routing_catalog` обновляется при закрытом Loader. При reopen production checkbox
уже false (desktop), query пуст, но виден прежний process catalog с PID helper.
И checkbox=false, и snapshot зафиксированы в final reopened record. Результат
не удаляет условия policy и не запускает сканирование сам по себе; однако
происхождение отображаемого каталога не соответствует новому экрану.

Минимальный отдельный product-контракт для root: request identity **и** identity
жизни Loader, переданные через реальный routing_catalog command и возвращённые
в result; GUI отображает только result текущей сессии/режима/запроса. Закрытие
инвалидирует право отображения, но не обязано отменять уже начатое blocking IO.
Process snapshot не переиспользуется как desktop result. Это потребует изменения
product command/snapshot/QML контракта и **здесь не реализовано**. Diagnostic
observer token для этой цели использовать нельзя. До решения root finding остаётся.

## Build, проверки и воспроизведение

Одна новая GUI build tree: 1.85.1, wrappers пустые, jobs1, with-rust-slot,
directory flock, offline/locked, собственный transient. После первого запуска
выявлена ошибка fixture seed: production v2 storage требует backup. Исправлен
только seed через существующий backup_closed API и выполнена incremental
перелинковка в том же tree (21.54 s), без новой полной build tree или установки.
Именно исправленный retained ELF используется final runs; ранние seed-failure
логи и старый P03 ELF не являются новым runtime доказательством.

Проверки: clippy двух затронутых packages all-targets gui,test-support -D warnings;
normal desktop --lib --no-default-features check; focused catalog tests;
Rustfmt, Ruff, Python A08 tests, shell syntax и diff-check. Точные результаты
и hashes закреплены в runtime-bindings. Независимого review child не было:
root запретил nested; выполнен scoped self-review разрешённого diff.

Replay: compile tools/a08_helper.c и shared tools/a08_gui_observer.cpp имеющимися
C/Qt компиляторами; подготовить own xdg/applications (native marker, wrapper,
oversized), empty-apps и проверенную Xvfb copy. Сохранённый campaign-runner.py
показывает четыре точных запуска. Внешний unshare --kill-child и timeout 35 s
ограничивают tree lifetime; shell trap завершает только свои helper/Xvfb.
До cargo clean сохранены ELF/source/logs вне transient. Cleanup outcome записан
в runtime-bindings; ссылки build tree не нужны для replay сохранённого ELF.
