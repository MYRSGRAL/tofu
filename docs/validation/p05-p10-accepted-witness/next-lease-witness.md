# Design only: actual ProxyLease replacement / ACK / window

Основа: merge `3ef474ee56bdb4bb3324d015d6d6028311a040f1`; это предложение,
не разрешение запуска, не новый runtime proof и не новая authority.

## Доступные существующие producers

- Обычный DesktopHost, без `with_test_subscription_fetch`, без ephemeral key для
  persistence: собственный Secret Service, synthetic repository и pinned core.
- `Command::Connect` компилирует выбранный профиль, увеличивает session_generation,
  создаёт private `Running { lease, cancel, task }`. `session()` выполняет
  BinarySnapshot/hash → CheckedConfig → LocalSession.start/check → SessionUpdate.
  Только Connected + !cancel + task alive публикуют endpoint в proxy_session.
- Default DesktopFetch захватывает actual lease/DNS; watch revoke отменяет fetch,
  удаляет manual pending, вызывает scheduled.shutdown/replacement/load. Это
  производители результата, а не fixture authority.
- `Stop`, затем новый `Connect` после окончания старого drain используют обычный
  command admission. Connect во время refreshing возвращает busy: обхода нет.
- Существующие set_subscription_policy/with_schedule_observer дают logical policy
  и applied/loaded barrier. Observer **не** выдаёт window_until/lease/token/ACK.
  SourceChange ACK — ACK изменения source, не замена session/scheduler drain ACK.
- Для внешнего oracle доступны snapshot generation/status/revision, реальные
  held upstream socket close, число запросов, persisted schedule через существующий
  Service metadata API после close. Обычный snapshot не доказывает private lease
  tuple или точный внутренний ACK сам по себе.

## Предлагаемый bounded сценарий после отдельного решения root

1. Подготовить свой TLS/SOCKS peer/upstream + synthetic profile, pinned binary
   hashes; NormalHost/private SS. Выставить persisted due через существующий API.
   Получить Connected реальным core и positive subscription request по default
   transport, не публиковать `(17, endpoint)` и не инжектировать Running.
2. Через обычные настройки разрешить background; logical policy online/unmetered/
   !power_saver, hidden UI. Дождаться applied policy barrier и первого actual
   scheduled held response. Зафиксировать t0 первого запроса; внутренний window
   мог начаться раньше — t0 не объявлять точным window_until.
3. Послать Stop. Удерживаемый upstream должен наблюсти EOF; старое поколение не
   публикует review/commit, revision остаётся рабочей. Дождаться нормального
   Disconnected/drain перед новым Connect. Socket EOF отдельно не достаточен для
   полного matching scheduler ACK: сопоставить с завершением owner и admission
   нового запроса, не считать один status доказательством всех ACK.
4. Reconnect тем же listen endpoint при новом generation, если существующий port
   API допускает повторное владение. Negative: старый response не воскресает;
   positive: новый lease действительно передаёт запрос. Если обычный admission
   запрещает конкретный interleaving, фиксировать запрет, не обходить guard.
5. Проверить nonextension внешне: никаких новых auto requests после исходного
   bounded окна и до следующего persisted due; late response не коммитится,
   готовый review живёт до явного решения. Чтобы отделить «нет due» от «окно
   закрыто», нужен заранее due второй synthetic source, который дал бы запрос при
   ошибочном продлении, и positive control внутри разрешённого окна. Оба source
   делят deadline. Время измерять монотонно, без нового test clock/hook.
6. Close/reopen normal SS, сравнить due/retry/revision, завершить и reap весь
   собственный core/peer/bus/SS closure. Raw evidence только synthetic counts,
   booleans/digests, generation receipts; без URLs/credentials.

Это один стык, не повтор 124 cases. Не обещается точный private window deadline
по отсутствию HTTP. Если external controls не различают сохранённый deadline и
отсутствующий due, до реализации нужен root proposal на **read-only private test
observation** window origin/deadline + owner/drain identity. Текущий approval
concrete transport builder такого observer не включает; новый hook здесь не
добавляется. Existing cfg(test) ProxyLeaseFixture не substitute actual producer.

## Где N5 grant необходим, а где нет

На этом exact source LocalSession.start — существующий legacy LocalSocks producer:
прямой process::spawn проверенного core с listener ownership/readiness. Он не
вызывает ProductionPermit::mint; local Connected нельзя назвать N5 NativeRunning.
Такой отдельно разрешённый owned loopback witness может доказать только этот
legacy DesktopHost lease path, без TUN/system/native admission claims.

Если root требует witness именно нового protected worker/native lifecycle,
необходим accepted N5 issuer/grant и его actual production consumer перед
получением NativeRunning/Run authority. Нельзя разрешать запуск выдачей fake
permit: `worker/role_handoff.rs::ProductionPermit::mint` здесь fail-closed
`Err(WorkerError::Peer)`. Никакой новый token, NativeRunning, privileged session,
VM/TUN или worker path в рамках PR199 не предлагается. Root должен выбрать
legacy LocalSocks proof scope либо дождаться N5 и назначить другой authority
owner. N5 не prerequisite для уже выполненного PR199 fetch/Service witness.
