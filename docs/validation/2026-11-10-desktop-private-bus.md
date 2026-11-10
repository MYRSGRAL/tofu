# P10: DesktopHost и private system bus

База: `b6d2891fbdbafc001ba9908dff33f11aa7714576` (HEAD подготовленного worktree).
P10 целиком не закрывается. FR-06 и performance прочитаны вместе с отчётами
scheduler, policy review и persisted due. helper/network/session не изменялись.

`backend::p10_e2e::private_bus_desktop_gate` запускает отдельный subprocess
текущего Rust test binary. Только этому subprocess передаётся собственный
DBUS_SYSTEM_BUS_ADDRESS. Fake NM/UPower/PPD работают через существующий Python
PyGObject/Gio, настоящий dbus-daemon и Properties/GetAll/PropertiesChanged/
NameOwnerChanged. Нужны установленные dbus-daemon и Python с gi; тест не
устанавливает пакеты и не пропускается молча при их отсутствии. Новых Cargo
зависимостей нет. Все fetch направлены на собственный loopback listener;
encrypted SQLite и ключ [77;32] эфемерны. Глобальное окружение не меняется.
Штатный NetworkMonitor DesktopHost остаётся включённым: он читает netlink ОС,
но system bus направлен на fixture, host network не перенастраивается.

Единственное production изменение — доверенный builder
HostConfig::with_system_subscription_policy(bool). Его вызывают после
with_ephemeral_key: последний по-прежнему отключает provider по умолчанию.
GUI JSON не получает настройки policy; set_subscription_policy и приоритет
explicit override не менялись. Новый тест не вызывает set_subscription_policy.

RED: компиляция нового gate завершилась E0599 — builder отсутствовал.
Первый GREEN прошёл; затем сценарий усилен двумя реальными fetch.

Проверяемые сценарии:

- Startup: настоящий provider получает unknown metered=0 через GetAll;
  видимый DesktopHost не делает запрос. Subscribe-before-snapshot и отмена
  старого GetAll при racing PropertiesChanged отдельно проверяются существующим
  private-bus regression в yonik-linux, который входит в релевантный прогон.
- Metered=1 и power-saver с unmetered=2 блокируют новые запросы.
- ReleaseName NM блокирует запрос; RequestName возвращает владельца;
  balanced/unmetered при hidden продолжает блокировать запрос.
- Переход visible после reacquire допускает настоящий FetchHost HTTP request.
  До approve total=0; после approve total=1.
- Второй DesktopHost с подготовленным overdue переживает kill/wait daemon.
  Во время outage запросов нет, ожидание пересекает первую секундную retry;
  daemon запускается на прежнем socket path, сервисы регистрируются заново.
  После восстановления visible допускает второй fetch и review/approval.
- shutdown/wait_closed завершаются; ListNames private bus после каждого
  shutdown содержит ровно одно уникальное имя — сам fake provider. Клиентские
  соединения и связанные bus subscriptions больше не существуют.
- Reopen той же encrypted DB с future persisted due, healthy provider и visible
  не создаёт повторный запрос; повторный shutdown также оставляет одного клиента.

Граница clock: overdue подготавливается существующим persist_schedule API
между закрытыми DesktopHost. Рабочий DesktopHost использует настоящий clock;
будущий due после completion проверяется коротким окном наблюдения. Сутки
не проматываются. Расчёт rollback/remaining/full backoff 1..30 и healthy reset
подтверждают прежние детерминированные тесты, а данный интеграционный сценарий
проверяет реальный outage/recovery, без измерения всех retry интервалов.
Отрицательные проверки наблюдают listener 350 мс после 200 мс settling события;
это ограниченное окно регрессии, а не доказательство энергетического idle.
Qt/Wayland, реальная батарея, suspend/resume, PSS/CPU/wakeups/энергозамеры,
Android и полная приёмка P10 этим отчётом не заявляются.

Сырые RED/GREEN/validation logs вне git и TTL:
`/home/myrs/.local/state/codex-evidence/yonik-p10-e2e`.
Rust 1.85.1, jobs=2, target в собственном transient root под BSD flock.

Итоговые проверки кандидата:

- `cargo +1.85.1 test -p yonik-desktop -p yonik-linux -p yonik-subscription`:
  440 passed, 0 failed, 24 ignored; отдельный вывод child gate не считается дважды.
- `cargo +1.85.1 clippy --workspace --all-targets -- -D warnings`: exit 0.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check`: exit 0.
- Python fixture: compile и фактическое выполнение прошли; ruff отсутствует,
  пакеты не устанавливались. Gio сообщает existing API deprecation warning.
- Промежуточный усиленный сценарий выявил неверное ожидание fixture total=0
  во втором review после первого approval; исправлено на previous_total=1.
  Это не production finding. Остальных production fixes не потребовалось.
- Cargo.lock восстановлен после проверки семантического равенства TOML
  (Cargo изменял только порядок). Generated logs/archives не входят в git.
- После окончания Cargo/test процессов lsof не нашёл открытых файлов target;
  штатный cargo clean выполнен, собственный transient root удалён.

Независимый review и CI не подменяются этими локальными проверками.
