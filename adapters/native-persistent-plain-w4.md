# W4: persistent Plain для закрытых Go TUN adapters

Область: Go delta от PR126 `140e05893cce09d374eaecc2818f88201e29558d`.
Изменения Rust W4, PR151/148/142 и новый main в эту ветку не включены.

## Контракт

Закрытый Xray constructor в `proxy/tun/tun_linux.go` вызывает `newFdTun(fd, 0)`.
Закрытый sing-box constructor в `protocol/tun/inbound.go` передаёт FD в
`sing-tun.New` с `GSO=false`; pinned sing-tun
`v0.9.7-0.20261023115432-040db71fcd33`, `tun_linux.go`, оставляет `vnetHdr=false`
на FileDescriptor path. Оба читают Plain IP, без virtio header.

Оба `validateTUNFD` допускают ровно `0x1001` и `0x1801`.
`IFF_PERSIST` — свойство хранения интерфейса, а не packet framing.
VNET `0x5001`/`0x5801`, TAP, отсутствие NO_PI, MULTI_QUEUE и любые другие
сохранённые биты запрещены. Проверки identity/name/ifindex/MTU/CLOEXEC/nonblocking
и владения FD сохранены. Нет caller bool, downgrade или Vnet constructor.
ATTACH offset104 остаётся semantic `0x1001`; ABI260, pins, capabilities,
production issuer и feature acknowledgements не изменены.

## Проверка

`TestPersistentPlainTUNPipeline` повторно использует actual inbound pipeline
каждого ядра в отдельной root netns с поднятым loopback. Fixture действительно
выполняет TUNSETPERSIST и требует kernel TUNGETIFF `0x1801`, оставляя wire
`0x1001`. Для каждого ядра восемь циклов cancel/eof/revoke/stop проверяют
TCP и UDP echo через kernel TUN → actual constructor → routing → outbound,
затем закрытие writer, запрет дальнейшего traffic и возврат ресурсов.
Прежний nonpersistent fixture и тест сохранены отдельно.

`TestPersistentPlainActualTUNConstructor` проверяет actual constructor, отзыв
и сохранность helper original FD. Existing custody/metadata/one-use tests
проверяют ownership. `TestKernelPlainTUNValidation` проверяет реальные Plain,
persistent Plain, VNET10, persistent VNET10, VNET12, TAP, MULTI_QUEUE,
CLOEXEC и blocking FD. Header size задаётся pointer ioctl и считывается обратно.

На проверенном kernel PI request нормализуется в NO_PI, unknown bit `0x4`
удаляется. Эти случаи явно отмечены в логе: kernel rejection для них не заявлен.
`TestPlainTUNFlags` проверяет все 65536 u16 значений, включая отсутствие NO_PI
и сохранённые unknown bits. Vnet acceptance, foreign UID и production issuer
не проверены и не заявляются. Fixtures используют закрытый test issuer.

Sensitive controls: до изменения production validator новые persistent pipeline
тесты обоих ядер подтверждают `0x1801`, затем FAIL на ATTACH с ошибкой broker proof.

## Evidence и ограничения проверки

Kernel `7.2.9-1-cachyos`, Go `go1.27.1-X:nodwarf5 linux/amd64`.
Scoped race tests обоих cores (kernel negatives, persistent constructors,
persistent pipelines и existing custody/metadata/one-use) прошли; scoped
`go vet -p=1 -mod=readonly ./common/yonikbroker` прошёл для обоих cores.
Builder unit tests: 18/18; Ruff check и format check builder и его тестов прошли.
Gofmt и package/callsite hashes сверены для обоих patches.

Холодный штатный builder sing-box достиг лимита 180 секунд на codec tests.
При догреве полного suite наблюдался отказ неизменённого
`TestActualSocketHandoffCounter/tcp/known` из-за истечения broker proof.
Повтор штатного полного codec suite прошёл; отдельный baseline control этого
теста прошёл 5/5. Timeout и отказ сохранены, изменения в этот тест не вносились.

Бинарники, manifests, полные test/build logs, команды builder,
`*-sources.sha256`, controls и SHA256SUMS сохранены вне transient:
`/home/myrs/.local/share/codex-artifacts/yonik-native-plain-w4/`.

| Core | Pinned source commit | SHA256 final patched source manifest |
| --- | --- | --- |
| sing-box | `c005f35300ce76427dc285481ec5695f5b179cd2` | `0af162c1582cc4a233b632c127d4fa23d3236230567971d5c12f8d419e9f9ca0` |
| xray | `a4a3dff5b20c2faae567b742c172c7b95e75c083` | `7fa39fcaf0436d48c3d3ab67201176bf03f7c6a01b41cf9ee4962112b2113224` |

Полный Xray builder не прошёл: `TestNativeDNSConcurrentAuthority` повторяемо
падает на broker proof/coalescing. Тот же DNS-suite падает на контрольном
бинарнике со старым strict validator. DNS tests и production DNS code не
менялись; этот отказ не исправлялся в W4. Это ограничение полного acceptance,
а не зелёный полный suite. Отдельный direct-build manifest явно содержит
`full_builder_passed=false`; он не подменяет штатный builder manifest.

Полный штатный sing-box builder прошёл. Direct build Xray закреплённой командой
`go build -p=1 -mod=readonly -trimpath -buildvcs=false -o <binary> ./main`
и package tests `./app/dispatcher`, `./proxy/tun` прошли.

| Бинарник | SHA256 |
| --- | --- |
| sing-box | `8a1683227b52a94ef904f8cdfbb676b16f9cf4409e68c046d3a5c7acebe591b5` |
| xray | `3f884f3a11793b1de208e7d287485d5674837e58b30c350090669f253e9e43a2` |

После сохранения бинарников/evidence проверено отсутствие активных процессов
и открытых ссылок на task tree; собственные Go cache очищены штатным
`go clean -cache -modcache` с локальными GOCACHE/GOMODCACHE. Каталог
`/var/tmp/codex-myrs/transient/native-plain-w4` удалён; Rust builds не запускались.
