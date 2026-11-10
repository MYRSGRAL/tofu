# Первый переносимый срез scheduler подписок P10

Base checkout: `21aba59f7e161a67d98ebe7415315e3fb8afcf9a`.
Область: `yonik-subscription::scheduler`; Linux host и GUI не изменены.
P10 остаётся незавершённым до host integration и приёмки runtime.

## Контракт и выбранная политика

Чистая синхронная модель получает монотонное время `Duration` от host.
Нет runtime, I/O, threads, futures, таймеров на сервер и UI polling.
`next_deadline` позволяет host поддерживать единственное событие пробуждения;
при запрещённой политике или занятом лимите возвращает None до host-события.
Время должно относиться к одной монотонной эпохе экземпляра; после рестарта
host восстанавливает сроки из сохранённых метаданных, не сериализуя Duration.

ID подписки и source generation — i64, как у SubscriptionService/Repository.
Work содержит уникальный ID экземпляра scheduler, serial и source generation.
ID экземпляра задаёт host, без повторного использования при живых callbacks.
Это correlation token планирования, а не новый Ticket импорта или CAS snapshot.
Host запускает существующий service refresh и подтверждает complete после всей
операции, включая review/commit; ошибки и NotModified классифицирует host.
Политика допускает только online, unmetered и отсутствие battery saver.
Скрытый UI допускает функциональное обновление лишь в background_window host.
Открытие UI само по себе не добавляет проверок серверов или опроса статистики.
Ограничения одинаково консервативны на всех платформах, без ручного override.
Изменение политики откладывает новые работы; уже активные завершаются.

Одна запись на подписку, общий capacity включает pending, active, stopped после
ошибок и cancelled до acknowledgement. Лимит параллелизма задаётся отдельно.
При переполнении — явный Full, дубли — Duplicate. Обслуживание одного вызова
линейно по ограниченному capacity, хранение O(capacity); скрытой очереди нет.
Сортировка due затем ID даёт детерминированный порядок.

После успеха следующий автоматический refresh через 24 часа от завершения.
Начальный due передаёт host с учётом прошлой проверки. После ошибок задержки
1, 2, 4, 8 секунд, пятая ошибка останавливает retries до явного resume.
Функция задержки насыщается на 60 секундах даже при больших значениях счётчика.
Jitter не добавлен: это выбранная детерминированная политика обновлений данных;
требование jitter для восстановления VPN из performance.md относится к другому
lifecycle. Переполнение времени останавливает запись до resume, serial exhaustion
даёт ошибку без wrap. Resume не сокращает уже назначенный суточный интервал.

Отмена pending удаляет запись немедленно. Отмена active возвращает Work для
остановки host fetch; запись удерживает слот до complete, поэтому отмена не
позволяет превысить реальное число одновременно живых операций. После ack можно
добавить новый source generation; старый callback не изменит новую работу.
При смене source/delete host также обязан инвалидировать существующий service
request/CAS. Scheduler сам не изменяет Repository и не публикует snapshots.

## Проверки

Rust/Cargo 1.97.0; CARGO_BUILD_JOBS=2. Отдельный target под shared flock корня:
`/var/tmp/codex-myrs/transient/yonik-scheduler-t3-83ebe60a/target`.

- `cargo fmt --all --check` — успешно.
- `cargo test --locked -p yonik-subscription` — 6 scheduler + 5 lifecycle тестов.
- `cargo clippy --locked -p yonik-subscription --all-targets -- -D warnings` — успешно.
- `git diff --check` — успешно; зависимости и lock не изменены.

Scheduler-тесты проверяют due/суточный интервал, retry budget/cap, overflow,
resume, pending/active cancellation, stale/повторный callback, дедупликацию,
capacity/parallelism и события питания/сети/видимости без периодического UI опроса.
Отсутствие фоновой активности доказано только устройством чистой модели;
измерения wakeups/энергии и реальное отключение host polling ещё не выполнены.

## Оставшаяся интеграция

Один owner связывает scheduler с SubscriptionService, системными событиями и
разрешёнными окнами ОС. Нужны bounded fetch executor, таймауты/ack отмены,
сохранение последней проверки, корректное завершение при review, маппинг ошибок,
тесты смены source/delete и перезапуска host, аппаратная приёмка скрытого UI.
Реальных сетевых запросов, desktop build, Docker или deploy в этом срезе нет.
