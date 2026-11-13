# Native hooks: граница до Rust handoff

Primary source закреплён в source-lock.json. Существующие четыре patches
не изменены. Новый wire ABI и featureack в этой работе не записаны.

В `route/route.go` точками guard являются `routeConnection:72` и
`routePacketConnection:248`, до любого default/forced routing. Общий
`prepareMatchMetadata:572` вызывает `searchProcessInfo`, но текущий
`route/process_cache.go:38` возвращает void и при ошибке лишь пишет лог.
Этого недостаточно для app-required fail-closed: новый guard обязан возвращать
ошибку до dispatch и обходить upstream process cache/local-source heuristic.

`experimental/libbox/service.go:81` передаёт dup FD в tun.Options.FileDescriptor;
`:191` содержит FindConnectionOwner wrapper. Это in-process Go interfaces,
не закрытая native CLI transport authority. `protocol/tun/inbound.go:567/586`
получает исходные TCP/UDP source/destination. Исходный canonical tuple необходимо
сохранить до DNS/sniff. Existing matcher conjunction source-app AND destination
не сводится к отдельным OR rules. Non-app paths сохраняют destination semantics.

Внутренняя семантика proof отдельно тестируется в
`tests/isolatedfixture/nativehooks`, без wire/featureack и без новых dependencies.
При actual integration каждый DNS/dial использует Session.Context; Guard вызывается
перед routing, после DNS и перед новым dial; успешный Track перед публикацией
передаёт Session закрытие flow. EOF/revoke/broker death отменяет pending I/O и
закрывает все TCP/UDP flows. Одного закрытия TUN FD недостаточно.

До production подключения Root передаёт exact closed Rust schema, roles,
SCM_RIGHTS custody и bootstrap authentication. Нельзя копировать fixture136,
доверять caller path/UID/PID/env или открывать TUN по fallback. После handoff
нужны actual TUN и positive flow proof обоих ядер в helper VM; эти component
проверки не закрывают FR20 и не включают system_tun.
