"""Непривилегированный fixture endpoint/ledger; запуск сейчас не разрешён.

Конфигурация выводится только из fixed root-sealed authorization/fixture/grant/launch.
Отдельного caller config/argv/env executor нет. DIRECT/proxy различие обеспечивает
trusted topology, не этот responder. Модуль W1 загружается по fixed installed path.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import selectors
import socket
import time


def main() -> None:
    # Тот же fixed root seal reader W1; импорт модуля не измеряет host.
    spec = importlib.util.spec_from_file_location(
        "n5_root_collector", "/usr/lib/yonik-acceptance/collect_observations.py"
    )
    if spec is None or spec.loader is None:
        raise PermissionError("MISSING_ROOT_COLLECTOR")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    backend = module.SystemBackend()
    decode = module.decode
    root = "/usr/lib/yonik-acceptance/"
    auth = decode(backend.read(root + "n5-authorization.json", sealed=True))
    if (
        auth.get("schema") != "N5_AUTHORIZATION_V1"
        or auth.get("execution") != "AUTHORIZED"
    ):
        raise PermissionError("EXECUTION_CLOSED")
    values = {}
    for key, name in (
        ("fixture", "sealed-guest-fixture-v1.json"),
        ("grant", "n5-user-grant.json"),
        ("launch", "n5-trusted-launch.json"),
    ):
        raw = backend.read(root + name, sealed=True)
        if hashlib.sha256(raw).hexdigest() != auth.get(key + "_sha256"):
            raise ValueError("SEALED_PIN")
        values[key] = decode(raw)
    grant, launch = values["grant"], values["launch"]
    if (
        grant.get("explicit_user_runtime_grant") is not True
        or grant.get("fixture_sha256") != auth["fixture_sha256"]
        or grant.get("launch_sha256") != auth["launch_sha256"]
        or grant.get("source_commit") != values["fixture"]["source_commit"]
    ):
        raise PermissionError("GRANT_BINDING")
    controls = launch["controls"]
    endpoints = {}

    def address(value: str) -> tuple[str, int]:
        host, port = value.rsplit(":", 1)
        return host.strip("[]"), int(port)

    for flow in controls["flows"]:
        host, port = address(flow["endpoint"])
        identity = (host, port, flow["transport"])
        item = {
            "host": host,
            "port": port,
            "transport": flow["transport"],
            "server": flow["response"],
        }
        if identity in endpoints and endpoints[identity] != item:
            raise ValueError("ENDPOINT_IDENTITY")
        endpoints[identity] = item
    config = {
        "observer": address(controls["flow_observer"]),
        "endpoints": list(endpoints.values()),
    }
    selector = selectors.DefaultSelector()
    ledger: dict[tuple[str, str, str], dict] = {}
    sockets: list[socket.socket] = []
    for endpoint in config["endpoints"] + [
        {
            "host": config["observer"][0],
            "port": config["observer"][1],
            "transport": "observer",
            "server": "observer",
        }
    ]:
        family = socket.AF_INET6 if ":" in endpoint["host"] else socket.AF_INET
        kind = (
            socket.SOCK_DGRAM if endpoint["transport"] == "udp" else socket.SOCK_STREAM
        )
        server = socket.socket(family, kind)
        server.bind((endpoint["host"], endpoint["port"]))
        if kind == socket.SOCK_STREAM:
            server.listen(8)
        server.setblocking(False)
        sockets.append(server)
        selector.register(server, selectors.EVENT_READ, endpoint)
    while True:
        for key, _ in selector.select(timeout=1):
            server, endpoint = key.fileobj, key.data
            connection = None
            try:
                if endpoint["transport"] == "udp":
                    data, peer = server.recvfrom(1024)
                else:
                    connection, peer = server.accept()
                    connection.settimeout(2)
                    data = connection.recv(4096)
                if endpoint["transport"] == "observer":
                    query = json.loads(data)
                    record = ledger.get(
                        (query["token"], query["transport"], query["endpoint"])
                    )
                    result = record or {**query, "seen": 0, "server": "", "peer_ip": ""}
                    connection.sendall(
                        json.dumps(result, separators=(",", ":")).encode()
                    )
                else:
                    token = data.decode("ascii")
                    host = endpoint["host"]
                    address = (
                        f"[{host}]:{endpoint['port']}"
                        if ":" in host
                        else f"{host}:{endpoint['port']}"
                    )
                    identity = (token, endpoint["transport"], address)
                    previous = ledger.get(identity, {})
                    ledger[identity] = {
                        "token": token,
                        "transport": endpoint["transport"],
                        "endpoint": address,
                        "seen": previous.get("seen", 0) + 1,
                        "server": endpoint["server"],
                        "peer_ip": peer[0],
                        "peer_port": peer[1],
                        "observed_ns": time.monotonic_ns(),
                    }
                    response = f"{token}:{endpoint['server']}".encode("ascii")
                    if connection:
                        connection.sendall(response)
                    else:
                        server.sendto(response, peer)
                    if len(ledger) > 4096:
                        ledger.pop(next(iter(ledger)))
            except (OSError, ValueError, KeyError, UnicodeError):
                # Невалидный запрос не создаёт ledger record или PASS.
                continue
            finally:
                if connection:
                    connection.close()


if __name__ == "__main__":
    main()
