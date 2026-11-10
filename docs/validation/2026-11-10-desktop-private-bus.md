# P10: DesktopHost и private system bus, исправление PR60 P2

Исходная база P10: `b6d2891fbdbafc001ba9908dff33f11aa7714576`.
База исправления PR60: `8945aa5a99b97804ad0d2b8e5319317a9d47521c`.
Предыдущая версия отчёта переоценивала barriers и cleanup: Python ack означал
отправку сигнала, а kill только test subprocess оставлял descendants и child DB.
Эти результаты заменены описанной ниже приёмкой. P10 целиком не закрывается.

`cargo test -p yonik-desktop --test p10_gate` — исполняемый harness=false binary,
не ignored. В штатном CI `cargo +1.85.1 test --workspace --locked` запускает его.
Существующий Rust CI уже устанавливает dbus-daemon; workflow не менялся.
Python/PyGObject fixture удалена полностью. Rust fake NM/UPower/PPD находится
в yonik-linux::policy_test_support, доступен только с feature test-support,
которую desktop подключает через dev-features существующего path dependency.
Новых crates/версий/разрешённых зависимостей нет. В production default fake API
не экспортируется. Штатный policy provider, owner fencing/backoff, explicit
policy override и default ephemeral guard не менялись.

Parent создаёт единственное собственное временное дерево, назначает worker
его cwd и собственный DBUS_SYSTEM_BUS_ADDRESS. Child не использует inherited
YONIK_* flags/path, не выбирает и не удаляет чужое дерево. Parent удаляет свой
каталог после завершения всей собственной process group. Worker, dbus-daemon
и потомки находятся в отдельной group. Только отдельный harness становится
subreaper: TERM, 300 мс grace, затем KILL и bounded reap через waitpgid своей
pgid; общий deadline worker 45 секунд. Pipe reader завершается после уничтожения
всех group writers. Daemon ready ограничен 3 секундами; zbus fixture operations
3 секундами; observer 8 секундами. Неограниченных ack/clients/read ожиданий нет.

Регрессия timeout запускает worker, реальную encrypted DB и dbus-daemon,
потомка sleep, игнорирующего TERM, и зависает до ready. Parent получает marker
создания ресурсов, через 2 секунды завершает group, reaps descendants, проверяет
ESRCH для pgid и отсутствие своего дерева/DB. Временные пути создаёт сам parent;
child не принимает произвольные cleanup paths из окружения.

Trusted HostConfig observer получает копию effective scheduler policy и признак
успешной загрузки persisted schedule в начале рабочей итерации DesktopHost.
Observer не меняет policy, GUI JSON его не настраивает. Все negative проверки
стартуют после exact healthy hidden + loaded, затем ждут exact target policy и
visibility. Сигнал fake provider сам по себе barrier не является.

Проверены:

- healthy startup GetAll → effective online/unmetered/balanced + loaded;
- unknown metered=0, metered=1, power-saver при visible запрещают fetch;
- hidden при healthy запрещает fetch;
- NM owner loss даёт default forbidden, reacquire возвращает healthy;
- visible healthy после reacquire допускает реальный loopback FetchHost;
- review сохраняет прежний total, approve публикует новый snapshot;
- новый DesktopHost переживает kill daemon, запрещает fetch в outage, затем
  восстанавливает healthy после daemon restart и выполняет второй fetch/review;
- shutdown/wait_closed и ListNames оставляют только fake provider, без клиентов;
- reopen той же encrypted DB ждёт healthy+visible+loaded до проверки отсутствия
  повторного fetch с будущим due.

Deliberate delayed GetAll удерживается 600 мс и для unknown negative, и для
reopen. Отдельный waiter exact policy не завершается до освобождения GetAll;
после освобождения подтверждает applied state, только затем проверяется absence.
150 мс после barrier — окно наблюдения listener, не замена обработки события.
Fake fixture/worker запускаются с PATH, содержащим только dbus-daemon/sh/sleep;
python/python3 отсутствуют, а полный gate проходит.

Три mutation RED с последующим GREEN сохранены вне git:

- orphan: воспроизведён прежний kill только leader; pgid всё ещё существует,
  assertion fails. Перед failure исправленный group cleanup убирает все ресурсы.
- barrier: reopen принимает первое initial-unknown observation вместо exact
  healthy; delayed GetAll regression падает до освобождения снимка.
- dependency: прежний обязательный Python launch в PATH без Python падает ENOENT;
  Rust/zbus replacement работает в том же restricted PATH.

Clock: overdue готовится существующим persist_schedule API между закрытыми
hosts; рабочий clock не подменяется. Reopen не ждёт сутки. Полный backoff 1..30
и healthy reset покрыты прежними unit tests, gate пересекает первую retry при
реальном outage. Subscribe-before-snapshot race покрыта прежним private-bus
provider regression в релевантном прогоне. Qt/Wayland, suspend/resume, батарея,
энергия/PSS/CPU/wakeups и полная P10 приёмка остаются открытыми.
Штатный NetworkMonitor читает netlink, сеть ОС не меняется; system bus направлен
на fixture. Keyring/helper/session/network code не менялись.

Evidence вне git/TTL: `/home/myrs/.local/state/codex-evidence/yonik-p10-p2`.
Cargo: Rust 1.85.1, jobs=2, собственный transient root под BSD flock.

Финальная проверка исправления:

- `cargo +1.85.1 test -p yonik-desktop -p yonik-linux -p yonik-subscription`:
  439 passed, 0 failed, 24 ignored + отдельный p10_gate exit 0.
- `cargo +1.85.1 test -p yonik-desktop --test p10_gate --locked`: exit 0,
  повторён после последнего уточнения wait в hang fixture.
- `cargo +1.85.1 clippy --workspace --all-targets -- -D warnings`: exit 0.
  Первый clippy потребовал явный wait в намеренно зависающей fixture; worker
  теперь ждёт ребёнка, а parent timeout всё так же завершает и reaps group.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check`: exit 0.
- Cargo.lock семантически совпадает с базой и восстановлен без перестановок.
- После завершения проверок lsof не нашёл открытых файлов собственного target;
  cargo clean выполнен, собственный transient root удалён. Evidence сохранены.

Независимый delta review назначает root; CI результаты не подменяются локальными.
