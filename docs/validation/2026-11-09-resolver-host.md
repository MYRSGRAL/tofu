# ResolverHost: read-only снимок и подписка

## Реализованный контракт

`read_resolver_snapshot` вызывает только `io.systemd.Resolve.DumpDNSConfiguration`
на `/run/systemd/resolve/io.systemd.Resolve`. `subscribe_resolver` использует только
`io.systemd.Resolve.Monitor.SubscribeDNSConfiguration` на отдельном Monitor socket
с `allowInteractiveAuthentication=false`. Сетевые/DNS настройки, cache, polkit
и системные службы не меняются. Нет polling или fallback к D-Bus PropertiesChanged:
для ряда DNS-полей `EmitsChangedSignal=false`.

Сохраняются глобальные/per-link servers/fallback, route-only/search domains,
DefaultRoute, DNSSEC/DoT, scopes и resolvConfMode. Nullable факты не заменяются
ложью или пустым списком; незнакомые поля/режимы сохраняются и отмечают неполную
интерпретацию. Debug скрывает адреса, зоны, SNI и неизвестные значения.
`incomplete=false` описывает интерпретацию схемы, а не готовность системного DNS.

Transport принимает NUL-framed JSON до 1 MiB, отвергает повторяющиеся ключи,
повреждённые/незавершённые сообщения и неверный режим continues. Начальный бюджет
не превышает 5 секунд и охватывает подключение, запрос и первый полный ответ.
После первого байта неполный frame получает абсолютные 5 секунд; ожидание нового
frame в idle не имеет таймера. Отмена, EOF, timeout и ошибка закрывают fd. Полные
сообщения из одного read сохраняются для следующего вызова. Ошибка не заменяет
последнее наблюдение новым успешным результатом.

Production endpoints фиксированы, проверяются защищённые родительские каталоги,
тип/inode Unix socket до/после подключения и SO_PEERCRED. Это проверка локального
источника, не доказательство origin network namespace и не полномочие Apply.
API требует Tokio runtime с I/O/time. Stamp содержит случайный идентификатор
соединения и checked sequence; разовые снимки имеют разные идентификаторы.
Stamp и последнее наблюдение не являются lease или гарантией живости daemon.

## Проверки

Приватные Unix socket fixtures проверяют bytewise/coalesced frames, границу
размера, duplicate/deep JSON, unknown/nullable конфигурации, отказ авторизации,
неизвестный метод, EOF/half-close, startup timeout, отмену idle/partial read,
абсолютный partial deadline, отсутствие idle timeout, trust и закрытие fd.
Они не изменяют DNS хоста и не подменяют production endpoints публичным параметром.

Независимое ревью обнаружило наследование дедлайна предыдущего frame неполным
хвостом. Регрессионный тест воспроизвёл Timeout до исправления. Теперь время начала
хвоста берётся из read, впервые доставившего его байты; готовый coalesced frame
остаётся допустимым после паузы потребителя. Полный stream на реальном resolved
не проверен: доступ запрещён текущей авторизацией.

Ревью CLI также обнаружило блокировку reactor синхронным stdout. До исправления
заполненный pipe удерживал процесс после SIGTERM. Теперь последовательный вывод
исполняется в отдельном thread, ожидание отменяется и ограничено 1 секундой;
thread не принадлежит блокирующему пулу Tokio и не join-ится при выходе CLI.
Нативная проверка release с заполненным pipe подтвердила завершение по SIGINT и
SIGTERM за 1,22 секунды от старта (сигнал через 0,2 секунды), без сигнала — за
2,02 секунды с учётом попытки вывести статическую ошибку. Число незавершённых
writer threads ограничено одним штатным и одним при выводе ошибки перед выходом.

Локально на Rust 1.85.1 пройдены 265 Rust-тестов (6 native/ignored не запускались
этим общим набором), строгий workspace Clippy, форматирование, release сборка,
Android workspace check и отдельный Android check диагностического примера.
31 Python-тест прежних инструментов также прошёл. GUI не менялся и новые
Wayland/X11, tray, HiDPI или power acceptance этим этапом не заявляются.

На целевом systemd v262 CLI без сохранения адресов/зон получил 11 конфигураций,
3 основных и 6 fallback server entries, 4 search/route-only domain entries,
`resolvConfMode=foreign`, `streamed=false`, `incomplete=false`.
`watch` вернул `PermissionDenied` (в исходном API —
`io.systemd.InteractiveAuthenticationRequired`); интерактивная авторизация не
запрашивалась. Это ожидаемый отказ, а не успешное наблюдение изменений.

```sh
cargo run --locked -p tofu-linux --example resolver_inspect -- snapshot
cargo run --locked -p tofu-linux --example resolver_inspect -- watch
```

Для сборки/запуска задаётся внешний `CARGO_TARGET_DIR` и удерживается `flock`
корня собственного build-каталога, как в README. CLI выводит только счётчики,
известный режим и статический код ошибки; SIGINT/SIGTERM передают отмену API.
Живая обработка сигналов длительной подписки на хосте не подтверждена из-за
отказа Monitor; отмена сокета покрыта приватными fixtures.

## Ограничения и версии

Dump требует API v259+, Subscribe — v258+ и разрешения на monitor.
Схема сверена с [Varlink Resolve v262](https://github.com/systemd/systemd/blob/v262/src/shared/varlink-io.systemd.Resolve.c),
[Resolve.Monitor v262](https://github.com/systemd/systemd/blob/v262/src/shared/varlink-io.systemd.Resolve.Monitor.c)
и [реализацией daemon](https://github.com/systemd/systemd/blob/v262/src/resolve/resolved-varlink.c).
Ограничения D-Bus signals описаны в
[официальном resolve1 API](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.resolve1.html).

`foreign` не доказывает управляемый системный split DNS. API не реализует DNS
queries, собственный resolver, cache invalidation, Apply/rollback или privileged
helper. Нет атомарного объединения с topology и authorization generation.
Существующий GUI NetworkMonitor пока не использует этот поток и не гарантирует
полноту DNS configuration signals. PrivateNetworkBinding и строгий DNS/Fake-IP
остаются закрытыми; G-DNS/G-LINUX не объявляются пройденными.
