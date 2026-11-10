# P10: persisted due

База: `61743a994e96c8d0c489e0c9a00d5a000a40e187` (`origin/main`).
P10 целиком не закрывается. Provider policy из PR58 не меняется.

ScheduleState v1 хранится как optional поле существующего encrypted source
payload; старые payload читаются с serde default. Новые таблицы, SQL migrations
и зависимости не нужны. Служебная запись сохраняет source generation и использует
существующий CAS generation + decrypted payload, прежние encryption/AAD.
State не содержит URL/credentials; opaque metadata token хранит CAS payload в
Zeroizing, публичные поля — id/generation/state.

Startup читает страницы по 64, максимум repository 20 000, без snapshot body.
Старые записи без state получают сохраняемую отсрочку 60 + id % 86341 секунд.
Новые источники получают сутки. Overdue допускается только существующей policy,
по одному fetch/review. State проверяет version, failures 0..5, согласованность
paused/due и горизонт до суток; пятая ошибка требует paused до явного resume.
Retry state экспортируется/восстанавливается pure Scheduler; расчёт backoff и
completion state общий для runtime и persistence. Approve/discard/304 сбрасывают
failures и назначают сутки от completion. Cancellation не увеличивает failures.

Wall clock переводится в монотонный deadline через injected wall/monotonic inputs.
При rollback remaining ограничен прежним интервалом (до суток), при startup
recorded/due нормализуются и сохраняются CAS: следующий restart не начинает
полную отсрочку заново. Overdue/скачок вперёд становится due сейчас. При overflow
монотонного сложения используется now; невалидные timestamps дают видимую ошибку
хранения, а не ложное persisted состояние.

Перед автоматическим fetch сохраняется checkpoint с тем же retry budget и due
на сутки от старта. Если сохранение не удалось, транспорт не запускается;
ошибка публикуется, ID убирается из автоматической очереди текущего runtime.
При ошибке completion persistence ID также блокируется. Ручной refresh получает
аналогичный checkpoint; служебные изменения не повышают source generation.

Snapshot commit и completion metadata — две транзакции. При аварии между ними
остаётся checkpoint: сохранённый snapshot остаётся целым, retry budget не
сбрасывается, повторная автоматическая загрузка откладывается максимум на сутки
от старта операции. Незавершённый review после аварии не утверждается. Очень
долгий review может пережить checkpoint due; после такой аварии overdue fetch
допускается по policy. Это осознанная recovery semantics, не атомарный commit.
Generation change обнуляет schedule нового ресурса; старый CAS не применим.
Удалённые ID не переиспользуются repository.

Проверки выполняются Rust 1.85.1, jobs=2, CARGO_TARGET_DIR под собственным transient
root с удерживаемым BSD flock. Fetch fixtures — synthetic loopback, БД и ключи
эфемерные; host keyring/helper/пользовательская сеть не используются. GUI/Qt не
пересобираются. Сырые логи вне git:
`/home/myrs/.local/state/codex-evidence/yonik-persisted-due-20261110`.

RED подтверждён для отсутствующих snapshot/restore API core. Reopen metadata,
completion/retry/clock/crash/storage regressions добавлены в этом срезе; отдельный
предреализационный RED для каждого из этих тестов не заявляется.

Итоговая проверка кандидата:

- `cargo +1.85.1 test -p yonik-subscription -p yonik-linux -p yonik-desktop`:
  432 passed, 0 failed, 24 ignored (namespace/VM gates не запускались).
- `cargo +1.85.1 clippy --workspace --all-targets -- -D warnings`: exit 0.
- `cargo +1.85.1 fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.

Промежуточный backend RED выявил CAS-гонку checkpoint и первичной записи due;
запись checkpoint перенесена перед spawn ручного refresh, все 31 backend
integration-теста затем прошли. Ошибочная fixture storage failure через close
клона заменена повреждением ciphertext собственной БД. В финальном прогоне
ошибок нет. Cargo.lock восстановлен после проверки семантического равенства
зависимостей: Cargo менял только порядок. Независимый review и CI не подменяются
локальными проверками. GUI, platform events, power/resume/background acceptance
и измерения энергии остаются за пределами среза P10.
