# Read-only topology и kernel route-get

## Что реализовано

`read_topology` снимает links, IPv4/IPv6 addresses, маршруты всех таблиц, policy
rules и nexthop objects/groups через RTNETLINK. Селекторы правил, ECMP и ссылки
на nexthop не заменяются собственным алгоритмом маршрутизации. Незнакомые
атрибуты сохраняются; `incomplete()` означает неполную интерпретацию атрибутов,
а не проверку ссылочной целостности или готовности Apply. В native fixture
получено 140 таких атрибутов: результат явно помечен неполным.

`lookup_route` запрашивает фактическое решение Linux с заданными destination,
source, UID, mark, TOS, интерфейсами и TCP/UDP-портами. Он не отправляет пакет.
Ошибка ядра сохраняет errno; blackhole/нет маршрута не подменяются успешным
DIRECT. Неоднозначный или частично интерпретированный результат не становится
доказательством VPN bind. Полные данные доступны вызывающему host; Debug скрыт.

Оба API требуют Tokio runtime с I/O/time и явную отмену. Общий deadline до
5 секунд охватывает все запросы без неявных повторов. Используются только GET
операции; production-код не запускает `ip`, shell или privileged helper.

## Проверки транспорта

- Sender проверяется по адресу `recvfrom`, не по `nlmsg_pid` в заголовке.
- Для unicast проверяются seq и port; multicast определяется отдельно. Настоящий
  kernel route notification может наследовать ненулевой seq отправителя:
  первоначальная классификация Malformed воспроизведена и исправлена на Changed.
- Multipart требует DONE; ACK=0 не считается данными или завершённым dump.
  DUMP_INTR, ENOBUFS/OVERRUN, truncation, повреждение, неверная последовательность
  и превышение лимита возвращают ошибку без частичного успешного снимка.
- Multicast подписка создаётся до запросов. Наблюдаемое изменение отклоняет
  всю попытку; дополнительный drain после разбора DTO закрывает последнюю фазу.
- Лимиты: 64 KiB datagram, 16 MiB wire data и 32768 сообщений на весь reader.
  Сокет NONBLOCK/CLOEXEC принадлежит OwnedFd; фоновых задач reader не создаёт.

Unit/protocol tests сначала воспроизвели восемь ошибок transport stub, затем прошли.
Синтетические модели проверяют оба семейства, unknown/flagged/duplicate attributes,
префиксы, UID ranges, ECMP, nexthop weights и byte order портов. Нативный fence-тест
в отдельном namespace проверяет queued event до GET и событие после DTO, отмену,
timeout и закрытие fd. Этот environment-dependent тест отмечен ignored по умолчанию.

## Нативная сверка

`network_topology_smoke.py` создаёт новый user/network/PID/mount namespace,
проверяет dev/inode и исходный единственный `lo`. Перед каждой мутацией допускает
только собственный namespace и точный набор созданных интерфейсов. `ip -j` служит
независимым oracle внутри стенда, а не production-зависимостью reader.

Пройдены 13 сценариев: topology с несколькими таблицами/rules/ECMP, UID, fwmark,
source, TCP- и UDP-порты, ECMP lookup, блокировка, отсутствие пути, nexthop group,
IPv6, повторное чтение после изменения/удаления, неверный ввод и timeout.
IPv6 и nexthop objects реально доступны; они не пропущены. SIGINT/SIGTERM
при ожидании stdin завершают и reap CLI. Reader не меняет topology: изменения
делает только проверенный namespace fixture.

Ревью обнаружило P2 в публичном `--worker-namespace`: вымышленные inode могли
обойти launcher. Этот аргумент удалён; публичный entrypoint всегда запускает
`unshare`, а worker вызывается внутренним bootstrap после создания sandbox.
Регрессионный тест воспроизвёл обход до исправления; новые тесты проверяют отказ
поддельного аргумента и обязательные namespace-флаги wrapper. Нативные сценарии
повторены после исправления; отдельное TCP policy rule отличает TCP от UDP.

```sh
cargo build --locked -p tofu-linux --example network_inspect
python tools/network_topology_smoke.py "$CARGO_TARGET_DIR/debug/examples/network_inspect" --evidence /путь/к/evidence
```

Build-каталог и запуск удерживают `flock` его корня, как в README.
Локальные metadata/logs: `local/validation/network-topology/native-guard-fixed/` и
`fence/native-fixed.log`; первоначальный отрицательный fence — `fence/native.log`.
Полные raw данные host namespace не публикуются в evidence.

## Незакрытые контракты

Ни Linux dump, ни чистый drain не являются атомарным lease между Validate и Apply.
Событие может возникнуть сразу после последнего чтения. См. первичный
[контракт dump consistency Linux](https://cdn.kernel.org/doc/html/latest/userspace-api/netlink/intro.html)
и [RTM_GETROUTE](https://docs.kernel.org/netlink/specs/rt-route.html).

Нужны отдельные ResolverHost/DNS snapshots, authorization/application generations,
план собственных ресурсов helper, проверки реального VPN пути и эффектов Apply,
rollback/recovery. `LOCAL_ENV` локального SOCKS не объявлен системным окружением;
`PrivateNetworkBinding` остаётся Unsupported. Dangling/cyclic nexthop references
отдельно не валидируются этим диагностическим DTO. Полный TUN/G-DNS не заявляется.
