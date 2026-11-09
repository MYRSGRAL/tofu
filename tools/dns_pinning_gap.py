"""Синтетическая проверка привязки route lookup и dial в закреплённом Xray."""

from __future__ import annotations

import argparse
import json
import logging
import os
import socket
import socketserver
import struct
import subprocess
import sys
import threading
from collections.abc import Iterator
from contextlib import ExitStack, contextmanager
from pathlib import Path

from dns_fixture import (
    DnsFixture,
    Reply,
    engine_process,
    validated_binary,
    verify_namespace,
)
from engine_compile import socks_reply, wait_listener
from engine_smoke import install_signal_handlers, run_sandbox
from policy_traffic import MarkerTcp
from traffic_echo import receive_exact

ALLOWED = "198.51.100.10"
BLOCKED = "198.51.100.20"
PAYLOAD = b"p" * 32


class MarkerServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    block_on_close = True

    def __init__(self, address: str, marker: bytes) -> None:
        self.marker = marker
        self.hits = 0
        super().__init__((address, 24081), MarkerTcp)

    def get_request(self) -> tuple[socket.socket, tuple[str, int]]:
        result = super().get_request()
        self.hits += 1
        return result


@contextmanager
def marker_pair() -> Iterator[tuple[MarkerServer, MarkerServer]]:
    with ExitStack() as stack:
        servers = []
        threads = []
        try:
            for address, marker in [(ALLOWED, b"A"), (BLOCKED, b"B")]:
                server = stack.enter_context(MarkerServer(address, marker))
                thread = threading.Thread(
                    target=server.serve_forever, kwargs={"poll_interval": 0.05}
                )
                thread.start()
                servers.append(server)
                threads.append(thread)
            yield servers[0], servers[1]
        finally:
            for server in servers:
                server.shutdown()
            for thread in threads:
                thread.join(timeout=3)
                if thread.is_alive():
                    raise RuntimeError("Marker thread не завершён")


def native_config() -> dict:
    return {
        "log": {"loglevel": "debug"},
        "dns": {
            "tag": "dns-bootstrap",
            "queryStrategy": "UseIPv4",
            "disableCache": True,
            "disableFallback": True,
            "servers": [{"address": "127.0.0.1", "port": 1053}],
        },
        "inbounds": [
            {
                "tag": "socks",
                "listen": "127.0.0.1",
                "port": 10808,
                "protocol": "socks",
                "settings": {"auth": "noauth", "udp": False},
            }
        ],
        "outbounds": [
            {"tag": "block", "protocol": "blackhole", "settings": {}},
            {"tag": "dns-direct", "protocol": "freedom", "settings": {}},
            {
                "tag": "direct",
                "protocol": "freedom",
                "settings": {"targetStrategy": "ForceIPv4"},
            },
        ],
        "routing": {
            "domainStrategy": "IPOnDemand",
            "rules": [
                {
                    "type": "field",
                    "inboundTag": ["dns-bootstrap"],
                    "outboundTag": "dns-direct",
                },
                {"type": "field", "ip": [BLOCKED + "/32"], "outboundTag": "block"},
                {"type": "field", "domain": ["full:pin.test"], "outboundTag": "direct"},
            ],
        },
    }


def connect_marker() -> str | None:
    try:
        with socket.create_connection(("127.0.0.1", 10808), timeout=1) as control:
            control.settimeout(2)
            control.sendall(b"\x05\x01\x00")
            if receive_exact(control, 2) != b"\x05\x00":
                raise RuntimeError("Неверный SOCKS greeting")
            name = b"pin.test"
            control.sendall(
                b"\x05\x01\x00\x03"
                + bytes([len(name)])
                + name
                + struct.pack("!H", 24081)
            )
            socks_reply(control)
            control.sendall(PAYLOAD)
            response = receive_exact(control, 1 + len(PAYLOAD))
            if response[1:] != PAYLOAD or response[:1] not in (b"A", b"B"):
                raise RuntimeError("Неверный marker payload")
            return response[:1].decode()
    except (OSError, EOFError, ConnectionError):
        return None


def worker(host_namespace: int, binary: Path, output: Path) -> None:
    verify_namespace(host_namespace)
    for address in (ALLOWED, BLOCKED):
        subprocess.run(
            ["ip", "addr", "add", address + "/32", "dev", "lo"], check=True, timeout=3
        )
    results = []
    for name, first, later, expected in [
        ("stable-allowed", ALLOWED, ALLOWED, "A"),
        ("stable-blocked", BLOCKED, BLOCKED, None),
        ("changed-between-route-and-dial", ALLOWED, BLOCKED, "B"),
    ]:

        def answer(domain: str, kind: int, index: int) -> Reply:
            if domain != "pin.test":
                raise ValueError("Неожиданное имя DNS fixture")
            if kind != 1:
                return Reply()
            return Reply((first if index == 1 else later,), ttl=0)

        case = output / name
        with marker_pair() as (marker_a, marker_b), DnsFixture(answer) as dns:
            with engine_process(binary, "xray", native_config(), case) as process:
                wait_listener(process, 10808)
                observed = connect_marker()
                alive = process.poll() is None
                hits = [marker_a.hits, marker_b.hits]
            reaped = process.returncode is not None
        if observed != expected or not alive or not reaped:
            raise RuntimeError("Native route/dial контроль не подтвердился")
        if not dns.records or any(record["qtype"] != 1 for record in dns.records):
            raise RuntimeError("Нет подтверждённого IPv4 lookup")
        if name == "stable-blocked" and hits != [0, 0]:
            raise RuntimeError("Заблокированный контроль достиг endpoint")
        if name == "changed-between-route-and-dial" and (
            len(dns.records) < 2 or hits != [0, 1]
        ):
            raise RuntimeError("Нет wire-proof второго DNS и другого endpoint")
        results.append(
            {
                "case": name,
                "dns": dns.records,
                "marker": observed,
                "hits": hits,
                "engine_alive": alive,
                "engine_reaped": reaped,
            }
        )
    result = {
        "engine": "xray-26.10.31",
        "gate_supported": False,
        "observation": "route_address_not_pinned_to_dial",
        "netns_isolated": True,
        "external_network": False,
        "cases": results,
    }
    (output / "result.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    )
    print(json.dumps(result, ensure_ascii=False), flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--worker", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    binary = validated_binary(args.binary, "xray")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if args.worker is not None:
        worker(args.worker, binary, output)
        return
    run_sandbox(
        [
            "unshare",
            "--user",
            "--map-current-user",
            "--keep-caps",
            "--net",
            "--mount",
            "--pid",
            "--fork",
            "--kill-child",
            "--mount-proc",
            sys.executable,
            str(Path(__file__).resolve()),
            str(binary),
            "--output",
            str(output),
            "--worker",
            str(os.stat("/proc/self/ns/net").st_ino),
        ],
        timeout=45,
    )


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    main()
