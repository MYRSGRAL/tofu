# L16: интегрированный bulk producer и сквозная приёмка

Предыдущий head: `6eaebaa6cfd29e03cce7143f17d8a299de370def`.
Из production изменены только `batch_diagnostics.rs` и batch-ветки `backend.rs`.
UI/QML/C++/main.rs, exporter producer, fetch/Proxy contracts не менялись.

## Владение, ограничения и fences

Actor немедленно создаёт один owned producer и возвращается в select. Producer
последовательно читает текущую source generation, вызывает repository bulk API
(один parse snapshot через существующий run/spawn_blocking), проверяет актуальные
source generation и snapshot revision, затем отправляет Prepared и Result.
В выходе только публичные поля и TcpProbeTarget; credentials/URI не передаются.

Core channel=16, GUI channel=16, parallelism=8. Ожидание места в GUI channel
поллится одновременно с core future: полный UI-выход не останавливает активные
TCP futures. Есть один текущий элемент конвертации; очереди не растут с n.
Публичная страница ограничена 32 строками, полный текущий результат — 10k.
Статус публикуется каждые 32 результата и на границах lifecycle.

При отмене actor посылает сигнал и закрывает/дренирует выход, сохраняя JoinHandle.
Producer не abort-ится. Bulk API может вернуть None раньше завершения blocking
worker, поэтому producer проходит через тот же repository gate/Mutex для ACK.
Этот проход использует существующий public load; новые общие helpers не нужны.
Actor остаётся доступен для приоритетных команд и освобождает producer после ACK.
Stop/Shutdown ждут этот owned producer, не оставляя CPU-работу detached.

Prepared/Result и терминальная очередь проверяются по lifecycle generation,
TCP cancel generation, source ID, snapshot revision и network stamp. Source
команды инвалидируют также уже завершённые страницы; rename проверен тестом.
Network invalidation закрывает выход, поздние Prepared/Result не восстанавливают
старые строки. Отмена не создаёт медиану для частичной/неуспешной серии.

## Проверки финального кода

- 20 desktop unit-тестов: 10k bounded DTO, полный output channel=16, отмена с
  сохранённым владением producer до ACK и остальные прежние unit-тесты.
- 34 backend-теста, включая ошибочные серии, страницы, credentials, отмену,
  смену источника и rename уже завершённого источника.
- Desktop clippy `--all-targets --no-deps -- -D warnings`: PASS. Для enum событий
  локально объяснён large_enum_variant: bounded 16 слотов вместо 10k Box allocations.
- fmt выполнен. Повтор полного workspace clippy из-за документации не делался;
  прежний внешний warning yonik-engine описан в предыдущем отчёте.

### ActualHost 10k — PASS

Реальный DesktopHost управляется JSON GUI protocol. Проверены все 10 000
уникальных profile ID и все страницы результатов: 10 000 reachable результатов,
по три успешные пробы, равенство набора result IDs набору импортированных IDs,
revision/network epoch и отсутствие synthetic credentials во всех snapshots.

16 собственных loopback listeners приняли ровно 30 000 TCP EOF, по 1 875 каждый.
Нет отправки payload и обращений к пользовательским подпискам. Разные destination
ports исключают концентрацию 30k соединений на одном endpoint/TIME_WAIT tuple.
От начала check_batch до чтения всех страниц: 9 943 мс (debug). Полный test suite
с импортом, обходом профилей и обоими gates — 45,91 с.

Повторная подготовка 10k отменена за 175 мс. Stop во время подготовки — 197 мс,
Shutdown — 174 мс. После каждого ACK счётчик EOF остался ровно 30 000:
подготовка не создала новые TCP-соединения после отмены.

Это actualHost нагрузка 10k, а не заявление о полном GUI load 10k.

### Реальный batch network epoch — PASS

128 целей, 16 listeners, первая серия завершена 384 TCP-соединениями. В
`unshare -Urn`, содержащем только lo, добавлены собственные маршруты TEST-NET
`198.51.100.0/24 dev lo` и `203.0.113.0/24 dev lo`. Никаких host routes.
Host namespace inode `4026531833`, тестовый `4026533532`.

RTNETLINK monitor ID `fbb2499bfeb223aba1c8cfebeaab3022`, epochs `1→2→3`.
Завершённые результаты очищены после первого события. Во второй серии при
следующем событии completed=32, суммарный accepted=468 (384 от первой серии),
то есть сетевой batch ещё выполнялся. In-flight результат инвалидирован,
последующие snapshots и страница подтверждения не возвращают старые строки.
Автоматический перезапуск не наблюдается. Отмена третьей серии — 5 мс.

### Qt user path связанного бинарника — PASS

Новый GUI binary собран с `--features gui` из тех же production blobs,
Rust 1.97.0 / Qt 6.11.2. Два настоящих Qt-запуска offscreen/software 420×760:
250 целей с видимым `250 / 250`, затем медленная loopback-серия с отменой
`0 / 250 · отменено`; оба завершены штатно. PNG просмотрены. Это user path,
а не заявление о GUI load 10k.

GUI SHA256: `66b3a60bd654280c755691abf3002564bfb6d5066891c1900ad90cc8aa140ef4`.
Acceptance SHA256: `b5004d3c931a89c7bc7f8a2b45a37ae8a68365f27870f3e3494892acb33bed72`.

## Evidence/provenance

Локальные исходные логи и PNG: `local/evidence/batch-integrated/` вне TTL:
`final-tests.log`, `final-gates.log`, `clippy.log`, `gui-build.log`, `qt/`.
SHA256 бинарников и Git blob SHA production-файлов — `provenance.json` там же.
Бинарники сохраняются вне build TTL, пути перечислены в manifest.

Старый FAIL 213,42 с не переписан и не выдаётся за PASS; он сохранён в
`local/evidence/batch-acceptance/network-run.log`. Новый actualHost gate выполнен
на интегрированном producer. API benchmark 895 мс остаётся отдельным измерением.

Оба ранее отсутствовавших runtime gates теперь пройдены. Независимый review
combined diff перед merge остаётся обязательным; merge/watch не выполняются.
