"""Фиксированный непривилегированный flow child. Исполняется только sealed driver."""

from __future__ import annotations

import hashlib
import importlib.util
import ipaddress
import json
import os
import socket
import sys
import time


def parse_endpoint(value: str) -> tuple[str, int]:
    host, port = value.rsplit(":", 1)
    address = str(ipaddress.ip_address(host.strip("[]")))
    number = int(port)
    if not 1 <= number <= 65535:
        raise ValueError("FLOW_PORT")
    return address, number


def normalize_flow(flow: dict) -> dict:
    result = dict(flow)
    host, port = parse_endpoint(result["endpoint"])
    result["endpoint"] = f"[{host}]:{port}" if ":" in host else f"{host}:{port}"
    for name in ("source_ip", "expected_peer_ip"):
        result[name] = str(ipaddress.ip_address(result[name]))
    return result


def read_frame(connection: socket.socket, deadline: float, limit: int) -> bytes:
    """Один bounded newline frame; каждый fragment использует исходный deadline."""
    data = bytearray()
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("FRAME_DEADLINE")
        connection.settimeout(remaining)
        chunk = connection.recv(min(4096, limit + 1 - len(data)))
        if not chunk:
            raise ValueError("FRAME_TRUNCATED")
        data.extend(chunk)
        if len(data) > limit:
            raise ValueError("FRAME_BOUND")
        if b"\n" in data:
            if data.index(10) != len(data) - 1:
                raise ValueError("FRAME_TRAILING")
            return bytes(data[:-1])


def exchange(flow: dict, nonce: str) -> dict:
    flow = normalize_flow(flow)
    endpoint = flow["endpoint"]
    host, port = parse_endpoint(endpoint)
    family = socket.AF_INET6 if ":" in host else socket.AF_INET
    transport = socket.SOCK_STREAM if flow["transport"] == "tcp" else socket.SOCK_DGRAM
    with socket.socket(family, transport) as client:
        client.settimeout(2)
        client.bind((flow["source_ip"], 0))
        local = client.getsockname()
        received, reply = False, ""
        try:
            deadline = time.monotonic() + 2
            client.connect((host, port))
            message = f"{flow['token']}:{nonce}".encode("ascii")
            if transport == socket.SOCK_STREAM:
                client.settimeout(max(deadline - time.monotonic(), 0.000001))
                client.sendall(message + b"\n")
                reply = read_frame(client, deadline, 1024).decode("ascii")
            else:
                client.sendall(message)
                reply = client.recv(1024).decode("ascii")
            received = True
        except TimeoutError:
            pass
        local_text = (
            f"[{local[0]}]:{local[1]}"
            if family == socket.AF_INET6
            else f"{local[0]}:{local[1]}"
        )
        return {
            "local": local_text,
            "remote": endpoint,
            "transport": flow["transport"],
            "received": received,
            "reply": reply,
        }


def sealed_controls() -> dict:
    spec = importlib.util.spec_from_file_location(
        "n5_root_collector", "/usr/lib/yonik-acceptance/collect_observations.py"
    )
    if spec is None or spec.loader is None:
        raise PermissionError("MISSING_ROOT_COLLECTOR")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    backend, decode = module.SystemBackend(), module.decode
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
        or os.geteuid() != launch["controls"]["caller_uid"]
    ):
        raise PermissionError("GRANT_BINDING")
    return launch["controls"]


def main() -> None:
    controls = sealed_controls()
    raw = sys.stdin.buffer.readline(4097)
    if len(raw) > 4096 or not raw.endswith(b"\n"):
        raise ValueError("BYTE_BOUND")
    value = json.loads(raw)
    if set(value) != {"flow", "nonce"}:
        raise ValueError("FLOW_PROTOCOL")
    value["flow"] = normalize_flow(value["flow"])
    if value["flow"] not in [normalize_flow(flow) for flow in controls["flows"]]:
        raise ValueError("FLOW_NOT_SEALED")
    print(json.dumps(exchange(value["flow"], value["nonce"]), separators=(",", ":")))


if __name__ == "__main__":
    main()
