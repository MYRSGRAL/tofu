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

## PR59: исправление трёх P2 после integration merge

База delta: `4e35eabb4290081ccab3a6cd9c7dd234ae349b69`; root merge с provider58
сохранён. Ownership delta: backend, subscriptions API, их tests и этот отчёт.

Ручная операция захватывает `SubscriptionIdentity { id, generation }` до
checkpoint. Та же identity передаётся fetch с expected generation, completion
metadata и последующему cancel/reload. Ручной pending review возвращает identity
самого PendingRefresh; approve/discard не используют текущий UI selection.
Перед reconcile scheduler проверяется поколение ресурса: старый completion не
отменяет очередь или review нового поколения. Checkpoint и completion API
проверяют expected generation до прежнего payload CAS; гонку после проверки
по-прежнему отклоняет CAS repository.

Владельцы fetch согласованы admission guards: Refresh получает busy, пока идёт
плановая работа. При существующем ручном/плановом review Refresh/Import/HWID
получают review_required до отмены scheduler и сохраняют отчёт. HWID продолжает
отменять свою active работу с acknowledgement; если после cancel остаётся чужой
fetch, новая ручная работа не запускается. Сам переход HWID сбрасывает schedule
в encrypted payload той же операции change_source, без последующего checkpoint
как условия корректности.

Три регрессии выполнены RED до изменения production-кода, затем GREEN:

- `p2_ручной_b_не_пересекается_с_плановым_a_и_не_теряет_review`: RED — ручной B
  запустился при удерживаемом loopback fetch A и вернул fetch error вместо busy.
  GREEN — guard отказывает, затем review A появляется, переживает повторный
  Refresh и завершается штатным discard.
- `p2_старое_manual_completion_не_изменяет_новое_поколение`: RED — completion gen1
  после change_source gen2 вернул Ok. GREEN — StaleUpdate, schedule gen2 остаётся
  None. Дополнительно проверен отказ старого checkpoint.
- `p2_hwid_сбрасывает_paused_в_переходе_поколения`: RED — gen2 наследует paused5.
  GREEN — непосредственно после set_source_hwid generation=2 и schedule=None,
  без запуска ручного checkpoint/fetch.

Rust 1.85.1, jobs=2, собственный transient root с BSD flock:
`cargo test -p yonik-subscription -p yonik-linux -p yonik-desktop` — 439 passed,
0 failed, 24 ignored; среди прошедших четыре provider-теста. GUI/Qt не собирались.
Сырые RED/GREEN и validation logs вне git:
`/home/myrs/.local/state/codex-evidence/yonik-pr59-p2-20261110`.
Clippy заметил непроверенный размер read только в новой loopback fixture;
проверка усилена assert > 0, затем повторены backend regression, workspace
clippy с all-targets/-D warnings и fmt check. Provider58, core scheduler,
network/helper/application catalog/QML и зависимости в delta не менялись.
Независимый delta review остаётся root; детей не запускали. P10 не закрывается.

Финальные backend regression, workspace clippy и fmt завершились exit 0.
Перед очисткой lsof не нашёл открытых файлов собственного target. Cargo target
очищен штатным cargo clean, собственный transient root удалён; evidence сохранён
в указанном каталоге без TTL. Сгенерированные логи не входят в commit.
