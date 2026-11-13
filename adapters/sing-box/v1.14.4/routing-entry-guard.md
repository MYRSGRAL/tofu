# Guard входа sing-box

Закреплённый upstream: `c005f35300ce76427dc285481ec5695f5b179cd2`.
`route/route.go`: `routeConnection`, `RoutePacketConnection`,
`routePacketConnection`, `matchRule` и `PreMatch` проверяют закрытый consumer
до detour, DNS hijack, выбора rule и default outbound. PreMatch при отказе
возвращает Reject; синхронный entry возвращает ErrDenied.

Используются frozen broker, authenticated reply, исходный tuple и session binding.
Unknown/Unavailable для app-required запрещены. Отмена после lookup или при
возврате authenticated reply запрещает routing, retire sequence и отзывает session.
Конфигурация и environment не создают authority. `export_test.go` предоставляет
fixture только тестовому бинарнику; production его не содержит.

Приёмка вызывает реальные TCP/UDP API. Known проходит entry TCP до detour loop;
отдельный Known проходит matchRule до default outbound, Unknown/Unavailable не
вызывают default. Нет handoff — TCP/UDP возвращают ErrDenied. Отдельный тест
cancel-during-lookup воспроизводит отказ и проверяет отзыв сессии.

Production issuer/relay и установка consumer пока не подключены, поэтому
production без trusted handoff закрывает routing. `runtime_hooks_connected=false`
сохранён; pins, capabilities, ABI не изменены. TUN injection, afterDNS,
и полная runtime цепочка остаются обязательными отдельными этапами.

## Guard перед socket dial/listen

`pinNetDialer` получает свежий `BeforeSocketDial` после control hooks для каждого
TCP/UDP connect, включая parallel-interface и late fallback. UDP `ListenPacket`
и serial-interface listen используют `listenGuardedPacket`: проверка до передачи
управления и повторная после control hooks. TFO сохраняет отмену исходного flow,
проверяет proof при initial Write и использует тот же control guard.

`TestActualSocketGuard` и `TestActualSocketFreshProof` вызывают реальные API
`DefaultDialer`: Known создаёт private loopback TCP/UDP socket; отсутствие handoff,
Unknown/Unavailable, stale epoch, expired, cancel и revoke запрещают выдачу сокета.
`TestActualDelayedTFODenial` меняет proof между возвратом deferred connection
и initial Write. Отказ proof отзывает session и её зарегистрированные ресурсы.
Wire, pins и capabilities сохранены. Это частичная приёмка перед dial/listen:
DNS after-resolve и полная сеть остаются обязательным отдельным этапом.

Счётчик lookup подтверждает новый proof вместо reuse routing proof.
TestActualSocketHandoffCounter использует native ProtectPath как счётчик передачи
реального socket FD: no-handoff/Unknown/Unavailable/stale/expired/cancel/revoke → 0,
Known → 1 для TCP, UDP connect и UDP listen. RED запускается на исходном codec.
Матрица реальных interface fallback в этой приёмке отсутствует. Transport-specific WireGuard/OpenVPN sockets,
использующие UDPListenerControl, требуют отдельного анализа; общая приёмка
DefaultDialer не доказывает их покрытие.
