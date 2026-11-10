# Linux runtime плановых обновлений — срез P10

База рабочего checkout проверена до изменений:
`e33fa1908dfb3deb5434506341674de0b3a827b4`.
Ветка: `t3/linux-subscription-scheduler`. Head реализации: `7a2833ecda03c2909f3d9a7cc932fb4b2c64bbe5`.
Следующий docs-коммит фиксирует этот SHA и нормализует завершающую пустую строку
лога; код после проверок не изменён.

Основание: Task 2 плана `docs/superpowers/plans/2026-11-10-spec-completion-waves.md`
(прочитан из основного repo), FR-06 requirements.md и P10 docs/PLAN.md.
Срез заканчивается публичным Linux API: desktop принадлежит другой задаче.
P10 целиком не принят: нужны GUI wiring, системные power/network events,
восстановление due из метаданных и измерения энергии на целевом оборудовании.

## Изменённые файлы

- `crates/yonik-linux/src/subscription_scheduler.rs`: исполнитель чистого Scheduler.
- `crates/yonik-linux/src/subscriptions.rs`: injectable транспорт, generation fence,
  освобождение завершённых запросов и явный discard неподтверждённого отчёта.
- `crates/yonik-linux/src/lib.rs`: экспорт адаптера.
- `crates/yonik-linux/tests/subscription_scheduler.rs`: изолированные тесты.
- Этот новый отчёт. fetch/repository/desktop/helper и зависимости не менялись.

## API для следующего GUI среза

`SubscriptionScheduler::new(identity: NonZeroU64, capacity, parallelism)` создаёт
одного владельца. Identity уникален среди одновременно живых scheduler.
`schedule(id, source_generation, due: Duration)` ограничивает общий размер очереди;
`resume(id, now)` явно возобновляет исчерпавшую retries запись.
Host передаёт монотонное время и `yonik_subscription::scheduler::Policy`.

На событие питания/сети/видимости или общий deadline вызвать
`dispatch(now, policy, &service, Arc<dyn SubscriptionFetch>)`.
Штатный транспорт: `Arc::new(FetchHost::new(timeout)?)`.
`next_deadline(policy)` возвращает один общий абсолютный deadline или None.
Futures запускаются при ожидании `next_event(clock)`; clock вызывается после
готовности результата, поэтому backoff отсчитывается от завершения.
Host включает это ожидание в select с системными событиями и одним таймером.
Ожидание next_event разрешено прерывать: futures остаются внутри адаптера.
Без ожидания next_event сетевые futures не продвигаются.

Событие возвращается по одному: `Review { id }`, `NotModified { id }`,
`Failed { id, error }`, `Cancelled { id }`. Оно не содержит URL/credentials/тело.
Нет unbounded канала событий, spawned task на запись или собственного таймера.
Bounded набор futures и отчётов ограничен parallelism; queue — capacity.

`review(id)` даёт ссылку на неизменяемый PendingRefresh для явного показа.
`finish_review(&service, id, approve, now).await` сохраняет только при approve=true.
Даже чистый импорт автоматически не утверждается. При false отчёт удаляется,
snapshot остаётся прежним. Повтор после прерывания ожидания продолжает уже принятое
решение, не меняя его approve. Host должен передавать тот же SubscriptionService,
который использовал при dispatch. Ошибки commit завершают работу с backoff.
Успех/отказ пользователя назначает обычный суточный интервал.
304 обновляет validators прежнего snapshot существующим путём сервиса.

Перед change_source/delete вызвать `cancel(id, now)`, затем штатную операцию
сервиса. Для active/review дождаться Cancelled, затем schedule нового поколения.
Для ещё не запущенной записи cancel удаляет её сразу без события.
Повторный schedule до acknowledgement возвращает Duplicate. Отмена review
проходит через future очистки и тоже требует acknowledgement.
`shutdown(now).await` отменяет все записи и дожидается очистки; host обязан
вызвать его до освобождения runtime. Drop обрывает futures, но не заменяет
штатный shutdown/ack. Уже явно утверждённый commit при shutdown дожидается
завершения; смена source/delete дополнительно защищена Repository CAS.

SubscriptionFetch — узкая доверенная граница в subscriptions.rs; конкретный
FetchHost в fetch.rs не изменён. Реализация обязана выполнять route/таймаут/отмену.
Штатный Proxy остаётся UnsupportedPeerVerification, без fallback DIRECT.
Для тестов используются synthetic error/hold transport и local loopback HTTP;
пользовательская сеть, подписки, keyring и helper не затрагиваются.

## Проверки

Все Cargo-команды: Rust 1.85.1, CARGO_BUILD_JOBS=2,
CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/yonik-scheduler-runtime/target,
под `flock -s /var/tmp/codex-myrs/transient/yonik-scheduler-runtime`.

Начальный RED: `cargo +1.85.1 test -p yonik-linux --test subscription_scheduler`
завершился отсутствием SubscriptionScheduler, затем API реализован.
Дополнительные сценарии добавлялись при разработке; не заявляется отдельный
RED каждого сценария. Сохранённый red.log содержит исходную ошибку.

Поведенческие сценарии: capacity, synthetic fetch и route Proxy, backoff от
управляемых часов, отмена до запуска/во время fetch, acknowledgement и reuse,
устаревшее поколение, review до approval, отказ с сохранением snapshot,
change_source/delete, shutdown queued/active/review. Успех проверяется реальным
локальным HTTP, без пользовательских данных. Qt не изменялся.

Финальные команды и результаты:

- `cargo +1.85.1 test --workspace`: exit 0, 531 passed, 0 failed, 24 ignored;
  шесть новых scheduler tests прошли. Штатные ignored требуют отдельных
  namespace/VM/аппаратных условий и не считаются подтверждёнными этим запуском.
- `cargo +1.85.1 clippy --workspace --all-targets -- -D warnings`: exit 0.
- `cargo +1.85.1 fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.

Логи сохранены рядом в `2026-11-10-linux-subscription-runtime/` до очистки
transient target. Cargo 1.85.1 пересортировал только локальные записи Cargo.lock;
автоматическая перестановка убрана, зависимости и lock в diff отсутствуют.
Отдельный Qt smoke не запускался: Qt ownership не менялся; workspace включает
существующие backend tests с временными БД, без пользовательских данных.

Остаток: независимый review root и CI PR; desktop integration, platform event
sources, persisted due и полная P10 матрица остаются отдельной задачей.
