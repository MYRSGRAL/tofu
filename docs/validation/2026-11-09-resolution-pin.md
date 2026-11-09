# Native DNS provenance и привязка direct-соединения

## Что добавлено

К закреплённому sing-box v1.14.4, commit
`c005f35300ce76427dc285481ec5695f5b179cd2`, последовательно применяются:

1. `strict-complete-dns.patch` — ошибка обязательного семейства не скрывается
   успешным ответом другого.
2. `resolution-provenance.patch` — optional LookupWithProvenance возвращает
   метаданные окончательно выбранного ответа до нормализации/rewrite TTL.
3. `resolution-pin.patch` — opt-in `pin_resolution` в route action `resolve`
   связывает direct TCP/UDP с исходным именем, портом, набором IP и дедлайнами.

Основная policy/application часть tofu остаётся Rust. Патчи не создают отдельный
DNS resolver или TCP/UDP engine: используются native transports, router и dialer.
Старые API/default/false сохраняют прежнее поведение. Продуктовый compiler пока
не включает смешанный DNS/Fake-IP: полного G-DNS здесь нет.

## Provenance

Оба пути DNS — explicit/legacy client и новый rule engine — сохраняют отдельный
результат каждого семейства: исходный QuestionName, конечный CanonicalName после
CNAME, адреса/успешный NODATA, monotonic Deadline, logical transport tag/type и
NativeEnvironmentHash. Последний — **хеш, не доверенное поколение**. Имена здесь
описывают native DNS; это не замена общего IDNA-контракта policy или права Apply.

Proof путешествует вместе с конкретным response/future через evaluate/respond,
named/anonymous race и terminal/pending. Спекулятивный или отвергнутый ответ не
подменяет provenance выбранного. Минимальный TTL включает релевантную CNAME-цепь
и SOA TTL/MINIMUM для NODATA. Нулевой/неизвестный срок, DNAME, некорректные
question/class/rcode/TC/opcode/Z отвергаются. Native transport tag не доказывает
фактический маршрут или приватный VPN upstream.

Provenance cache отдельный, создаётся лениво, ограничен 256 entries. Сообщения
больше 4096 wire-байт не сохраняются; Go objects дополнительно занимают память.
Возвращаемые списки копируются. Cache hit не продлевает Deadline; неподтверждённый
persistent/stale cache не становится proof. Rewrite TTL, local/DHCP/hosts/fakeIP,
predefined и неизвестные источники не дают происхождение для этого режима.
Фактическое потребление и масштабирование всего процесса ещё требуют профилирования.

Это факты конкретного результата, а не обещание его живости при возвращении:
дедлайн первого семейства может истечь, пока ожидается второе. Consumer проверяет
каждый срок перед использованием. Полнота относится к выбранной native-стратегии;
какие семейства требуются переносимой CIDR-политике, должен определить compiler.

## Pin до фактической отправки

`pin_resolution` включает StrictComplete и RequireProvenance. Immutable pin
копирует исходное имя, порт, множество адресов и дедлайны. Перед dispatch
проверяется соответствие metadata; override/sniff/подмена набора не приводят к
новому DNS под прежним outbound. В этом режиме proxy/custom ConnectionHandler
отклоняются явно: удалённое подключение этим компонентом не контролируется.

Проверяются serial/interface retries и syscall-adjacent Control hooks TCP.
Сохраняются прежние ControlContext/Control; после потенциально медленного hook
свежесть проверяется снова. Deadline dialer ограничен TTL, прежним deadline и
контекстом. Для delayed TFO срок действует через весь establishment context,
включая initial Write. Отмена таймера после успешного establishment не превращает
TTL в максимальную длительность уже установленного TCP.

UDP guard расположен на raw PacketConn под NAT и проверяет каждый фактический
endpoint/датаграмму. Batch/unwrap не раскрывают обход. При истечении нужен новый
flow; другой адрес/порт не разрешается по старому proof. Эти проверки не являются
атомарным lease ядра ОС и не подтверждают привязку к приватной сети.

## Нативные проверки

`tools/resolution_pin_smoke.py` всегда создаёт отдельные user/net/PID/mount
namespaces и /proc, проверяет parent/owned-interface identity перед ip-операциями.
Только loopback, собственная veth-пара и reserved addresses; host DNS/routes/firewall
и реальная подписка не меняются. Публичного worker/namespace override нет.
Manifest связывается с бинарником и всей упорядоченной цепочкой patch SHA256.
Выходной каталог новый; config-check, engine alive и reap обязательны.

На финальном бинарнике пройдены **62 сценария**: official upstream, patched
default/false и pin; direct TCP/UDP IPv4/IPv6, UDP connect/unconnected, TTL=0,
CNAME=0, NODATA, некорректный opcode/Z. Для межпопыточного истечения собственная
veth/ARP-задержка удерживает первый TCP адрес; baseline достигает второго после
TTL, pin не достигает marker. CNAME TTL и SOA.MINIMUM проверяются отдельно.
Состояние neighbor сбрасывается только на собственной pin-a перед каждым таким
случаем, чтобы предыдущая попытка не сокращала ожидаемую задержку.

Два cache-сценария — explicit и DNS rules — используют три отдельных TCP flow с
интервалами 0,7 с. A TTL=1 запрашивается дважды, AAAA NODATA TTL=60 — один раз;
второй cache hit не продлевает срок. Успех требует marker/точного payload и
server hits; отсутствие SOCKS-ответа само по себе не считается доказательством.
Для UDP после TTL первый payload достигает сервера, второй не достигает;
default/false сохраняет обе доставки. Domain reply SOCKS после обратного NAT
допустим; фактический IP подтверждается marker listener, а не типом ответа relay.

Дополнительно повторена прежняя 92-case complete A/AAAA матрица. Go unit и race
проверяют затронутые adapter/dialer/DNS/route/direct/option packages, включая
задержанный TFO Write, retry, metadata mutation, ownership UDP buffer, NAT и
невозможность batch/unwrap обхода. Cached-cookie TFO wire-сценарий отдельно не
воспроизводился: ограничение establishment context проверено unit и исходниками
закреплённой tfo-go/v2@v2.3.2. Rust/Qt source не менялись и GUI/power acceptance
этими проверками не объявляется.

Независимое ревью выявило ancestor DNAME, который обходил отказ и завышал TTL;
RED-тесты TTL 1/0 воспроизвели ошибку, после исправления они проходят. Дополнительный
header negative test воспроизвёл приём non-QUERY response, после исправления
opcode/Z проверены unit и native. В smoke исправлены ATYP=3 oracle и накопленный
ARP timing; первоначальные неуспешные evidence сохранены отдельно.

## Артефакт и границы

Финальный локальный бинарник:
`local/artifacts/sing-box-resolution-pin-header-v1.14.4/sing-box`.
SHA256: `15e16d14f9ea9dd2bbecd68eb5344ddb06ac59dc9093da97183300fc349e3831`.
Manifest содержит полную цепочку, build settings и `strict_dns_gate=false`.
После добавления reserved-Z unit case повторная сборка дала тот же binary SHA256;
переносимая побитовая воспроизводимость этим не доказывается. Desktop продолжает
использовать официальный engine. Evidence: `local/validation/resolution-pin/`.

Доверенные network/resolver/policy epochs, согласование с NetworkHost/ResolverHost,
привилегированный Apply/rollback, private VPN binding, proxy/Xray parity и Fake-IP
ledger/restart всё ещё обязательны. Смена сети/политики не защищена одним TTL/IP
pin, а transport hash не заменяет её наблюдение. Native metadata не является
подписанным capability manifest. Полная ResolutionLease и D-01…D-07 не заявляются.

Первичные контракты: [DNS headers](https://www.rfc-editor.org/rfc/rfc1035.html#section-4.1.1),
[negative TTL](https://www.rfc-editor.org/rfc/rfc2308.html#section-5),
[первоначальный sing-box client](https://github.com/SagerNet/sing-box/blob/v1.14.4/dns/client.go),
[TFO implementation](https://github.com/database64128/tfo-go/blob/v2.3.2/tfo_linux.go).
