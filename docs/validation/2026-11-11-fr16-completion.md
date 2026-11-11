# FR16: actual busy/loss, negative cleanup и контекст событий

Слой сохранения: `f51aa997831890ad07dfd599fb5d5c7f1b7d7d63`, PR106.
Зависимость: PR103, `aa9566c7d427a2df680a4e2523a722d61692727e`;
обычный merge в task-ветке: `d703156476214418dd64be537573e157a18a7c5f`.
C++/bridge/main и target владельца FR19 не изменены этим delta.

## Контракт и реализация

`data-and-contracts.md` §5 требует event-id, session-id, generation, тип и
ограниченную полезную нагрузку. Контекст обязателен. `EventLog::read_scoped`
и `ScopedPage` добавляют его рядом с прежними `LogEvent`/`LogPage`; прежний
`read()` и его экспорт сохранены. Счётчики, cursor, 2000 записей и 256 результатов
не увеличены. Контексты хранятся в параллельном ограниченном ring под тем же
mutex и возвращаются только для выбранных событий.

`LocalSession::with_event_log` получает отдельный session-id из журнала.
При успешном `Session::begin` новый immutable handle получает generation
из настоящего application Token (`yonik-application/src/lib.rs`, begin).
Это поколение плана сессии; транспортный счётчик отмен DesktopHost не используется.
Повторный start того же LocalSession получает новое поколение. Ранее выданные
handles сохраняют исходные session/generation; mutable global tag отсутствует.
Desktop DTO передаёт числа строками, экспорт включает только статические коды
и числа. Старый consumer может продолжать использовать read без контекстов.

CleanupFailed журналируется после принятого application-перехода; поведение
освобождения ресурсов и approved fault hook PR103 не меняется. Сохраняется один
ProcessExited: значение >=0 — exit code, <0 — отрицательный номер Unix-сигнала.
Отдельное событие сигнала отклонено регрессией прежней последовательности.

Файловый I/O перенесён в `spawn_blocking`. SaveJob допускает только одну работу
на host; копируется только ограниченный allowlist export принятой страницы.
Начальный matching `saving` оставляет GUI pending; terminal ACK завершает его.
Actor/Qt не выполняют open/write/fsync/rename и продолжают принимать Stop.
Закрытие host дожидается принятой работы; старый результат не заменяет более
новую страницу. Публикация сохраняет NOFOLLOW/EXCL/NOREPLACE, mode 0600 и fsync;
неограниченная очередь заданий отсутствует. Лимит относится к количеству работ
и байтам; время ответа файловой системы не гарантируется.

## Доверенный actual-стенд

Отдельный example `logs_negative` требует gui,test-support. Использует прежние
bridge/application.cpp, approved HostConfig.with_cleanup_fault, ephemeral key и
normal pinned BinarySnapshot::load. Нет env/GUI команды для инъекции или обхода
pins. LogsTest пропускается serde и доступен только через Rust API; durable host
не выдаёт EventLog. Удержание mutex/actor имеет deadline; stdin control принимает
только фиксированные действия. Каждый процесс запускается в собственной группе;
отрицательный engine exit выполняется через pidfd собственного дочернего core.
Finally снимает fault, освобождает ring/actor gate, вызывает настоящий shutdown
и очищает собственную группу до reap её лидера.

Queue-full создаётся удержанием callback настоящего actor и заполнением его
128 slots доверенными Rust запросами. Следующий GUI logsRefresh отвергается
настоящим DesktopHost::dispatch. Test bridge вызывает production dispatch_error
helper на частичном snapshot; фактические Main.qml и LogsView принимают matching
busy ACK. После освобождения старые queued ACK не заменяют busy страницу;
явный retry принимает новую страницу. Это не stub dispatch QtTest.

## Доказательства

Evidence вне TTL: `local/evidence/fr16-completion/`; бинарники:
`local/artifacts/fr16-completion/`. Source manifests записаны до сборок.

- context-red.log: отключён настоящий generation — immutable context test FAIL;
  job-red.log: снят single-job guard — второй save проходит вместо busy, FAIL.
  Это mutation controls после реализации, не initial TDD RED.
- context-green-final.log: 5 PASS, один прежний native opt-in ignored.
- green.log: 34 desktop library + 41 backend + 4 dispatch + 1 save ACK PASS;
  backend работает под private D-Bus, Rust 1.85.1, jobs=1/global Rust slot/flock.
- negative-pidfd: actual Qt/Main/LogsView/DesktopHost, 33 шага, closed=true.
  loss.json: 256 строк, evicted=17, rejected=1, missed=17, cursor=273.
  session-events.json: session 1/2, generation 1, CleanupFailed,
  CleanupConfirmed, ProcessExited=0 и ProcessExited=-9.
  Контроль переполнения очереди, mutex busy, partial matching ACK, old ACK fencing,
  retry, отказ save с сохранением файла и clipboard входят в этот прогон.
- QtTest: 6 PASS. RU/EN checker и Python syntax checks проходят.

Промежуточные неудачные native-прогоны сохранены: служебная stage-отметка
затиралась snapshot; затем supervisor ожидал trace UI-действия, хотя C++ trace
пишется на backend snapshot; затем неверно предполагалось comm=xray вместо memfd.
Исправлены только синхронизация и identity-проверка стенда. Положительные и
отрицательные native-сценарии используют реальные закреплённые cores.

## Открытые границы

Physical Plasma/GPOWER не выполнены; полный FR16/Pxx acceptance не объявляется.
Независимый delta review должен оценить этот PR и dependency PR103; локальные
проверки не заменяют review. Никаких изменений user keyring, сетевых настроек,
production pins, worker/helper/private IPC не выполнялось.

Финальные проверки кандидата: clippy Rust 1.85.1 с gui,test-support/all-targets
и -D warnings прошёл; C++ заголовки Qt/GCC дают прежние build-script warnings.
Production binary собран без test-support. `positive/{xray,sing-box}/trace.jsonl`
содержит по 52 шага и closed=true: real pinned start/stop/reap, явное сохранение
и отказ повторной записи, clipboard, скрытая панель, RU/EN. Ruff 0.16.6 найден
в уже имеющемся локальном cache (ничего не устанавливалось): format/check нового
Python-стенда и check обоих журналирующих fixtures проходят.

Перед очисткой pre-clean-references.json подтвердил отсутствие процессов с
cwd/exe/fd/mmap внутри task-каталога. Binary/evidence сохранены вне TTL.
Штатный cargo clean удалил 9994 файла (6.0 GiB); пустой transient root удалён.
