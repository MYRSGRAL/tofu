# SOURCE coordinator N5

API: `run_owned_guest(core, case)`. Импорт не выполняет I/O. Селекторы не
предоставляют полномочий; nonpositive возвращают UNAVAILABLE/SOURCE_ONLY.
Реализация основана на design `914d382`, база `86a936a`.

Установленное root-owned дерево Python и его import closure должны предоставляться
trusted provisioning. Coordinator не устанавливает себя, не создаёт grants и не
запускает CLI. Runtime authorization читается из фиксированного
`/usr/lib/yonik-acceptance/n5-authorization.json`, вместе с fixture/grant/launch;
root capture authorization/input — из `/run/yonik-acceptance`.
JSON читается через bounded openat/O_NOFOLLOW с проверкой всей цепочки, mode 0444,
single-link и pre/post inode metadata. Artifact modes берутся из проверенного
fixture. Primary GID определяется только из trusted `/etc/passwd`, без NSS.

Порядок: unit closure → fixed app-UID driver → baseline → Start → Await →
connected → Stop → CloseDesktopOwner → stopped → Cleanup → actual check_receipt.
Schema=2 и receipt v2 сохранены. Host.close и kernel cleanup завершены до
stopped_since и close progress. FR16 и SOURCE_INVARIANT/QML unavailable не меняются.
Непустой cgroup не заменяется ClosedHost или exit driver.

Один serial publisher executor; каждая публикация ограничена campaign/case
сроком и 15 секундами. До каждой публикации и spawn sealed gate проверяется снова.
После spawn контролируются owned child и отсутствие unsolicited stdout между
командами; actual UID/GID/groups/caps/exe/start проверяются до Start.

Exclusive directory flock удерживается до конца. Durable `lifecycle-owned`
создаётся до первой публикации; существующий marker или output запрещает новый
запуск. Никакого overwrite, retry или resume. При любом неизвестном результате
следующая команда не посылается; normal Stop не повторяется. `Quarantined.owner`
и process-local retainer удерживают future, executor, dirfd/lock и child/pidfd.
Late None не снимает UNKNOWN. При неподтверждённом reap pidfd/pipes сохраняются.
Marker намеренно остаётся и после успешного witness.

Внешний владелец VM обязателен: Python retainer не переживает смерть процесса.
Marker запрещает reuse, но не заменяет внешний quarantine management. После
ошибки VM нельзя уничтожать или использовать повторно до authorized разбора.
Coordinator не использует StopUnit и не сигнализирует PID из receipt.

Проверки SOURCE: pure Python mock tests в `test_coordinator.py`,
`test_phase_exchange.py`, existing `test_runner.py`; Rust example pure tests,
Rust 1.85.1 locked/offline/jobs=1, fmt/clippy; Ruff 0.17.4, normal Python и -O.
Никакого запуска guest, driver executable, Host, native, worker, DBus или probes
эти проверки не требуют. Артефакт driver только собирается и сохраняется.

Runtime prerequisites: root review; trusted installed source/import closure и
обновлённые runner/driver/source/manifest/launch pins; оба explicit sealed grants;
fresh owned guest; root-owned local passwd с primary GID; отсутствие inherited
ambient/file capabilities; внешний durable VM quarantine owner; реальные root
captures, normal lifecycle и witness. SOURCE tests не являются acceptance PASS.
