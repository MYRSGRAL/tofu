# N5 guest driver: исходная реализация, runtime CLOSED

Установочный путь будущего артефакта:
`/usr/lib/yonik-acceptance/n5-guest-driver --receipt-protocol=n5-v1`.
Регистрация, установка и runtime этой задачей не выполняются.

Entry point всегда отказывает до чтения stdin, файлов, D-Bus и запуска Host.
Verifier sealed USER grant/source/fixture ещё MISSING; переменная окружения,
аргумент или receipt не могут создать VerifiedFixture. Конструктор намеренно
отсутствует. Текущий runner также остаётся CLOSED.

`session.rs` компилирует реальные команды normal DesktopHost: ready,
set_engine, select_source, select_profile с revision, System mode, Connect,
Stop и shutdown с bounded ожиданием. Профиль выбирается из существующей БД,
не импортируется и не регистрируется. HostConfig не меняет ключ, backend или
SystemSession: используется обычный Secret Service и обычный helper path.
Private Secret Service должен быть отдельно обеспечен и запечатан fixture;
его проверка/создание не реализованы и не подразумеваются.

Очередь snapshots ограничена 64 сообщениями по 64 KiB; overflow запрещает
наблюдение. Unknown остаётся Unknown. Stop одноразовый, повторный Apply после
Stop запрещён. Drop выполняет только shutdown своего Host и не выдаёт receipt.

Публичный Connected — прямое наблюдение desktop status. Это НЕ прямое событие
admission, attach ACK или health worker. Source invariant composition может
стать отдельным обоснованием после ответа R2; текущий код не выдаёт его за событие.

MISSING: sealed grant/fixture verifier; canonical owned child/reap и network
restoration observation; coverage и внутренние admission/role/ACK receipts;
sealed controls для tampering/revoke/host-loss/cancellation cases. Соответствующие
поля typed observation равны null, не нулю. Нет нового privileged observer,
SourceProfile/RunRole issuer или direct SystemClient Apply fallback.

Строгий decoder принимает только 2 core × 9 case, 7 фиксированных action,
целочисленные schema/sequence, запрещает неизвестные и дублированные поля,
replay/overflow и oversized frame. Он ещё не подключён к runtime stdin:
это требует согласования минимальных наблюдений и sealed fixture с root.
Реализация не удовлетворяет прежнему внутреннему PREFIX контракту runner
и не объявляет 18 actual cases пройденными.

## Решение R2 и протокол v2

Новый helper observer не нужен. Внутренние full report/custody/roles/ACK могут
быть SOURCE_INVARIANT (inference), привязанным к actual System Connected и
trusted executing source closure. RAW_TRACE, B/C, report hash не заявляются.
Runner теперь отвергает schema 1, принимает только schema 2 с явными классами
ACTUAL/SOURCE_INVARIANT/UNAVAILABLE. ACTUAL обязательно для packet policy,
app owner flows, trusted tuple, reap/cgroup/network cleanup и чужих ресурсов.
Проверка JSON не аутентифицирует capture: нужен отдельный trusted verifier,
который пока отсутствует; execute остаётся CLOSED. Исторические 18 IDs сохранены,
но unsupported negative interleavings возвращают UNAVAILABLE/SOURCE_ONLY.

Session до Connect требует enabled executable rule в существующем routing
snapshot: тот же predicate использует InlinePolicy::requires_attribution.
Без app policy запуск отвергается; LocalSocks fallback не добавлен.
Pending: sealed verifier и настоящие packet/cleanup captures. Это исходный
adapter и строгий контракт, а не завершённый actual runtime witness.
Linux/P12–P17 оси вне наблюдавшихся flows остаются открытыми.
