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
