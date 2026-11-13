# Xray: authenticated DNS и фактический egress

Продолжение PR126 от `18eaf7bd1bf4a71457c8505c7c71c78ee23b225b`.
Изменён только Xray native adapter. Pins, capabilities, ABI260 и Rust не менялись.

`TestAuthenticatedDNSConcurrentAuthority` использует сохранённый private wire
issuer и held DNS reply. Проверены 18 случаев: TCP/UDP, Known, cancel, revoke,
epoch, Unknown, Unavailable, stale, wrong-session и wrong-binding. Первый caller
теряет authority, второй сохраняет её; оба ожидают общий question. У Known
проверяется увеличение authenticated request sequence после await. Затем каждый
caller проходит RoutingEntry и native DialSystem к точному адресу ответа
127.0.0.1. Счётчик effective SystemDialer фиксирует реальные вызовы default dialer
и успешную передачу socket, echo endpoint подтверждает payload `echo`.
Known: 1 dial / 1 socket handoff / 1 echo; denied: 0 / 0 / 0.
Смена глобального dialer внутри каждого echo не используется.

P2 cache-negative исправлен: `TestNamedDownstream/cache/*` меняет authority из
синхронного DNSCacheHit log checkpoint внутри queryIP, после entry guard.
Проверяются достижение checkpoint и отсутствие нового DNS question.
`TestAuthenticatedCacheFence` дополнительно прогревает настоящий DNS cache,
успешно выполняет RoutingEntry, меняет private issuer на Unknown именно в
DNSCacheHit checkpoint и требует ошибку без адресов.
Sensitive negative control удаляет только CheckAfterDNS в LookupIPWithContext:
тест достигает cache hit и падает с адресом `[127.0.0.1] <nil>`.
Это контроль утечки адреса; отдельная мутация socket registration не выполнялась.

Last-waiter/retry cancellation проверены под race: upstream context завершён,
exchange map и pubsub subscriptions освобождены. Authenticated fixture при
cleanup отзывает session и требует пустой flow registry. Реальные echo sockets
закрываются после получения payload. DNSExchangeContext копирует mutable outbound
caller: объединённая race-проверка обнаружила конфликт DNS dial с blackhole,
который устраняется отдельным outbound для DNS I/O.

Проверки: объединённая namespace race-приёмка (authenticated, cache fence,
named downstream), lifecycle race, broker package race, Go vet, Go build,
builder18, pinned Ruff 0.17.4 check / format --check, gofmt и git diff --check.
Evidence, тестовые бинарники, source archive и SHA256 manifest:
`/home/myrs/projects/yonik/local/evidence/dns-neighbors-e5a27/` (файлы actual).
Rust не запускался; exact remote Rust CI остаётся merge gate root.
Sing-box, Android и физические G-UI/G-POWER сюда не входят.

## Дополнение: sensitive actual socket ownership control

`TestAuthenticatedSocketOwnership` закрывает оставшийся пробел: authenticated DNS
возвращает 127.0.0.1, Known проходит RoutingEntry и реальный DialSystem TCP/UDP,
счётчики default dialer и echo подтверждают 1 dial / 1 handoff / 1 egress.
После этого cancel либо revoke обязаны завершить revokeDone, освободить flow
registry, запретить Write/Read на открытом сокете и последующий DialSystem.
Ожидание revokeDone необходимо: Revoke очищает registry перед закрытием FD.

Точная RED-мутация в `transport/internet/system_dialer.go` заменяет только вызов
OwnSocket и возврат ownedFlowConn на `return conn, nil`; BeforeSocketDial остаётся.
RED: TCP/revoke и UDP/revoke падают на разрешённом subsequent Write;
cancel падает по deadline завершения revoke (callback регистрации отсутствует).
Возврат исходного callsite: все четыре случая GREEN под race без SKIP.
Это отдельный контроль actual ownership, не cache fence.

Evidence: `/home/myrs/projects/yonik/local/evidence/xray-socket-control/`:
`ownership-red.patch`, `red.log`, `green.log`, `green-back.log`, оба race binary,
исходники GREEN и SHA256 manifest. Проверены Go vet, release build, builder18,
pinned Ruff check/format и scoped gofmt. Rust/ABI/pins/capabilities не менялись;
Rust проверки относятся к отдельной работе root. Sing-box остаётся следующим leaf,
его PASS этим результатом не заявляется.
