# Бюджет памяти подготовленных планов helper

Ограничивается объём удерживаемых конфигурационных buffers и sealed executable
images внутри одного процесса helper. Это учёт reservations/владения данными,
не измерение RSS и не прохождение G-POWER.

| Ресурс | Предел |
| --- | --- |
| Все конфигурационные buffers и их reservations | 64 MiB |
| Payload одной конфигурации | 4 MiB |
| Зарезервированная ёмкость одного factory / retained buffer | 8 MiB |
| Все учтённые executable images | 256 MiB |
| Один executable snapshot | 128 MiB |
| Stores в общем config pool | 16 |

Пулы процессные и разделяются production instances. `memory_usage()` показывает
reserved bytes; они могут включать ещё не заполненную зарезервированную память.
Счётчики не используются как permission token или доказательство applied-сети.

## Конфигурации

До auth/factory резервируется 8 MiB. Сериализация настоящего CheckedConfig
проверяет предел 4 MiB до reserve/extend и использует fallible reserve.
`retained_bytes()` сообщает реальную ёмкость Vec, а не только длину. После factory
излишек reservation освобождается; retained ёмкость не может превышать 8 MiB.

`claim_for_apply` возвращает `ClaimedArtifact<A>`. Владелец содержит данные и lease,
не предоставляет Clone, DerefMut или извлечение naked artifact. Данные удаляются
раньше освобождения учёта. Claim, удаление pending entry и закрытие store не
освобождают бюджет уже выданного владельца. Отказ, cancellation, stale attempt,
forget_peer и Drop освобождают только соответствующие собственные данные.

Production A обязан быть immutable после factory и точно сообщать удерживаемую
ёмкость; `PreparedArtifact` требует Send + 'static. CheckedConfig содержит private
bytes. Factory должен быть кооперативно отменяемым и не оставлять неучтённую
отсоединённую работу после завершения future.

## TTL и общий pool

Свой store очищает expired pending до резервирования. Если общий pool всё ещё
занят, он вызывает зарегистрированные reclaimers других stores и повторяет
резервирование. Callback list lock отпускается заранее, State берётся через
try_lock: конкурирующий claim не создаёт инверсию mutex. Выданные владельцы не
участвуют в TTL cleanup.

Pool хранит только Weak callbacks; callback хранит Weak State. Store удерживает
свой callback, поэтому нет цикла владения. Регистрация ограничена 16 живыми
stores; stale Weak записи удаляются при следующем обращении. Фонового polling нет.

## Executables

TrustedRegistry проверяет текущие directory/source metadata до reuse. У каждого
engine один serialized weak cache, привязанный к полной identity источника:
dev/ino, размеру, владельцу, mode/nlink, mtime/ctime с nanoseconds. Изменённый
источник проверяется заново, без обхода SHA-256/прав/NOFOLLOW.

Cache hit создаёт dup FD того же sealed inode и разделяет lease. Weak cache сам
не удерживает image после последнего snapshot. Кэши разных Registry не объединены,
но общий процессный executable pool ограничивает и такие отдельные загрузки.
Сервис должен переиспользовать Registry текущей установленной revision.

Перед memfd copy резервируется metadata size. Если источник вырос, бюджет
увеличивается до записи очередного chunk; нехватка завершает загрузку ошибкой.
После проверки хеша/seals reservation уменьшается до actual bytes. Lease живёт
со всеми shared images/snapshot FD. Обычный непривилегированный load(path) сохраняет
прежний режим без helper pool.

Активный host обязан удерживать BinarySnapshot до завершения/reap своего engine:
одни открытые helper FD не измеряют память внешнего процесса после их закрытия.
Существующий LocalSession удерживает snapshot; managed host/recovery ещё нужны.

## Остаток приёмки

Декодирование D-Bus и временный compiler AST не входят в этот учёт. Нужны
реальный service/IPC, ограниченный factory, recovery, hardening и измерения
G-POWER. Эти пределы не объявляются общей готовностью P12 или продукта.

Контракты: [реестр](helper-trusted-registry.md),
[подготовленные запросы](helper-prepared-requests.md).

Для sing-box config-budget учитывает две физические копии: исходный sealed config
и worker-owned sealed memfd для повторного открытия `/dev/stdin` ядром. Второй
reserve устанавливается в WorkerPayload до передачи FD; retained_bytes включает
обе копии. SystemWorkerSupervisor удерживает payload до подтверждённого empty
cgroup и завершения transfer. Отмена/timeout сохраняют custody до recovery.
Если supervisor уничтожен без empty proof, reserve и payload остаются в
консервативном карантине до выхода helper (memory budget остаётся занят).
Публичные лимиты не увеличены. Worker дополнительно резервирует локальный budget
до создания memfd; streaming pread не меняет offset исходного FD, временный buffer
zeroize, mode=0600, CLOEXEC и seals сохраняются. Xray использует прежний stdin FD.
