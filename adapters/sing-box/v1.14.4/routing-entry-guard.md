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
beforeDial и полный revoke остаются обязательными отдельными этапами.
