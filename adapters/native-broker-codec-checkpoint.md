# Checkpoint Go codec для NativeBroker v1

Замороженный ABI: `260bc50eeb4f3a40ac97cd9a54ff0ea943a833ef`.
Оба `native-broker-codec.patch` добавляют одинаковый компонент в
`common/yonikbroker` pinned sing-box v1.14.4 и Xray v26.10.31.
`tools/build_native_adapter.py --native-codec` применяет патч, сверяет SHA256
каждого файла и запускает его тесты до сборки ядра. Этот режим не подключает
компонент к runtime и не включает featureack, capabilities или native pins.

Проверяются frozen Rust vectors, границы сообщений, reserved bytes, FD count,
canonical tuple, обязательный OWNED_TUN, app-required feature matrix, binding,
монотонные request IDs, консервативный RTT/TTL, отсутствие cookie в запросах,
отзыв TCP/UDP flows и конкурентный Release/Revoke.

## Зависимость от Root для runtime integration

Нужен production issuer/relay API, который до exec payload выпускает
`YNKROLE1` exact192B/exact2FD: NATIVE_CHANNEL и SERVER_PIDFD. Issuer связывает
trusted helper peer PID/UID/starttime и live pidfd с session/generation,
invocation, epoch/digest и передаёт trusted record только Run worker.
Relay обязан обслуживать HELLO/FEATURES/ATTACH/ACK, давать helper-owned уже
bound TUN duplicate и проверять fixed worker credentials на каждом сообщении;
LOOKUP обращается к app-UID broker, REVOKE/EOF закрывает весь session scope.
Нужен executable acceptance driver этого producer для обоих cores.

Go consumer должен проверить inherited descriptors и record, проверить server
credentials/liveness на каждом recvmsg, затем внедрить owned TUN и guard в
реальные call sites до routing, после DNS и до dial. Xray openFromEnv не является
источником authority. Наличие codec не доказывает эти свойства.

Runtime hooks, owned-TUN injection и helper VM end-to-end acceptance пока не
реализованы этим checkpoint. Rust исходники и общие контракты не изменялись;
MSRV 1.85.1 `cargo fmt --all --check` и
`cargo clippy --locked -p yonik-linux --all-targets -- -D warnings` прошли.
