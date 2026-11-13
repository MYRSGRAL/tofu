# Guard входа Xray dispatcher

Патч к upstream `a4a3dff5b20c2faae567b742c172c7b95e75c083` подключает
`common/yonikbroker.RoutingEntry` к `Dispatch`, `DispatchLink` и
`routedDispatch` до forced tag, `PickRoute` и default handler.
Синхронные входы возвращают отказ; повторный отказ после sniff закрывает link.

Закрытый consumer хранит исходный tuple, session и binding. Lookup отправляет
frozen запрос, проверяет authenticated reply через существующий Linux consumer,
request ID и binding, уменьшает TTL на полный RTT. Unknown/Unavailable,
отсутствующий handoff, отмена и revoke запрещают routing. Произвольные env/config
не могут установить закрытый context key.

Production issuer/relay и установка consumer в контекст пока отсутствуют.
Поэтому production бинарник в этом режиме запрещает каждый Dispatch.
Положительный тест использует исключительно закрытый private fixture;
это не подтверждение работающего producer или end-to-end app routing.
`runtime_hooks_connected=false` обозначает отсутствие полной runtime цепочки;
добавленный `routing_entry_guard_connected=true` отмечает только этот guard.
Pins, capabilities и wire ABI не изменены. После DNS, dial, TUN и sing-box
остаются отдельными обязательными этапами.

## Guard перед системным dial

`DialSystem` (оба вызова) и каждый TCP Happy Eyeballs attempt используют
`guardedSystemDial`: новый authenticated lookup исходного closed flow и
`BeforeDial` непосредственно перед effective dialer. Default dialer повторяет
проверку перед `DialContext` и UDP `ListenPacket`, после подготовки сокета.
Контекст остаётся закрытым; routing proof не кэшируется для dial.

`TestActualSystemDialGuard` вызывает настоящий `DialSystem`, заменяя только
системный dialer счётчиком. После успешного routing fixture меняет ответ на
Unknown/Unavailable/stale/cancel/revoke: оба варианта sockopt оставляют счётчик
нулевым, как и отсутствие handoff. Known даёт два dial и два новых lookup.
Матрица выполняется для TCP и UDP. `TestActualLoopbackSocket` дополнительно
создаёт реальные loopback TCP и UDP сокеты через default dialer.

Это доказательство общего socket dial path, не полноты DNS: contextless
`LookupForIP`, freedom UDP resolver и DNS side effects требуют следующих задач.
Production issuer остаётся закрытым, поэтому production runtime без handoff
отказывает. Полная runtime цепочка и capabilities по-прежнему не заявлены.
