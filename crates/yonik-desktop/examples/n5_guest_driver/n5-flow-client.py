"""Фиксированный непривилегированный flow child. Исполняется только sealed driver."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import socket
import sys


def exchange(flow: dict, nonce: str) -> dict:
    endpoint = flow["endpoint"]
    if endpoint.startswith("["):
        host, port = endpoint[1:].split("]:")
    else:
        host, port = endpoint.rsplit(":", 1)
    family = socket.AF_INET6 if ":" in host else socket.AF_INET
    transport = socket.SOCK_STREAM if flow["transport"] == "tcp" else socket.SOCK_DGRAM
    with socket.socket(family, transport) as client:
        client.settimeout(2)
        client.bind((flow["source_ip"], 0))
        local = client.getsockname()
        received, reply = False, ""
        try:
            client.connect((host, int(port)))
            client.sendall(f"{flow['token']}:{nonce}".encode("ascii"))
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
    if value["flow"] not in controls["flows"]:
        raise ValueError("FLOW_NOT_SEALED")
    print(json.dumps(exchange(value["flow"], value["nonce"]), separators=(",", ":")))


if __name__ == "__main__":
    main()
