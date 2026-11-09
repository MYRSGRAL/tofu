"""Изолированное доказательство partial A/AAAA fail-open sing-box 1.14.4."""

from __future__ import annotations

import argparse
import json
import os
import socket
import struct
import subprocess
import sys
import threading
from collections import Counter
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator

from dns_fixture import (
    DnsFixture,
    Reply,
    engine_process,
    validated_binary,
    verify_namespace,
)
from engine_compile import socks_reply, wait_listener
from engine_smoke import install_signal_handlers, run_sandbox
from traffic_echo import receive_exact

IPV4 = "198.51.100.10"
IPV6 = "2001:db8::10"
MARKER = b"SYNTHETIC_PARTIAL_DNS_ROUTE"
SOURCE_REFS = [
    "https://github.com/SagerNet/sing-box/blob/v1.14.4/dns/client.go#L474-L497",
    "https://github.com/SagerNet/sing-box/blob/v1.14.4/dns/router.go#L1000-L1019",
    "https://github.com/SagerNet/sing-box/blob/v1.14.4/route/route.go#L709-L713",
    "https://github.com/SagerNet/sing-box/blob/v1.14.4/route/route.go#L924-L947",
    "https://sing-box.sagernet.org/configuration/route/rule_action/#resolve",
]


@contextmanager
def marker_servers() -> Iterator[list[str]]:
    """Оба адреса уже назначены loopback изолированного namespace."""
    stop = threading.Event()
    received: list[str] = []
    sockets: list[socket.socket] = []
    threads: list[threading.Thread] = []

    def serve(listener: socket.socket, address: str) -> None:
        while not stop.is_set():
            try:
                connection, _ = listener.accept()
            except socket.timeout:
                continue
            with connection:
                received.append(address)
                connection.sendall(MARKER)

    try:
        for family, address in [(socket.AF_INET, IPV4), (socket.AF_INET6, IPV6)]:
            listener = socket.socket(family)
            sockets.append(listener)
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind((address, 24081))
            listener.listen()
            listener.settimeout(0.1)
            thread = threading.Thread(target=serve, args=(listener, address))
            threads.append(thread)
            thread.start()
        yield received
    finally:
        stop.set()
        for thread in threads:
            thread.join(timeout=3)
        for listener in sockets:
            listener.close()
        if any(thread.is_alive() for thread in threads):
            raise RuntimeError("Marker server не завершился")


def native_config() -> dict:
    return {
        "log": {"level": "debug", "timestamp": False},
        "dns": {
            "servers": [
                {
                    "type": "udp",
                    "tag": "fixture",
                    "server": "127.0.0.1",
                    "server_port": 1053,
                }
            ],
            "final": "fixture",
            "disable_cache": True,
        },
        "inbounds": [
            {
                "type": "socks",
                "tag": "probe",
                "listen": "127.0.0.1",
                "listen_port": 10808,
            }
        ],
        "outbounds": [{"type": "direct", "tag": "marker"}],
        "route": {
            "rules": [
                {"action": "resolve", "server": "fixture", "strategy": "prefer_ipv4"},
                {
                    "ip_cidr": ["198.51.101.0/24", "2001:db8:ffff::/48"],
                    "action": "reject",
                },
                {
                    "ip_cidr": [f"{IPV4}/32", f"{IPV6}/128"],
                    "action": "route",
                    "outbound": "marker",
                },
                {"action": "reject"},
            ]
        },
    }


def connect_marker(name: str) -> tuple[bool, str | None]:
    try:
        with socket.create_connection(("127.0.0.1", 10808), timeout=3) as connection:
            connection.settimeout(5)
            connection.sendall(b"\x05\x01\x00")
            if receive_exact(connection, 2) != b"\x05\x00":
                raise RuntimeError("Неверный SOCKS greeting")
            encoded = name.encode("ascii")
            connection.sendall(
                b"\x05\x01\x00\x03"
                + bytes([len(encoded)])
                + encoded
                + struct.pack("!H", 24081)
            )
            socks_reply(connection)
            return receive_exact(connection, len(MARKER)) == MARKER, None
    except (OSError, ConnectionError) as error:
        return False, str(error)


def worker(host_namespace: int, binary: Path, output: Path) -> None:
    verify_namespace(host_namespace)
    for address in [f"{IPV4}/32", f"{IPV6}/128"]:
        subprocess.run(
            ["ip", "addr", "add", address, "dev", "lo"], check=True, timeout=3
        )
    cases: list[dict] = []
    for label, good_a, good_aaaa, higher_block in [
        ("both_servfail", False, False, False),
        ("both_valid", True, True, False),
        ("a_valid_aaaa_servfail", True, False, False),
        ("aaaa_valid_a_servfail", False, True, False),
        ("higher_aaaa_block", True, True, True),
    ]:
        directory = output / label
        directory.mkdir(parents=True, exist_ok=True)

        def answer(name: str, qtype: int, query_index: int) -> Reply:
            if name.rstrip(".") != "partial.test":
                return Reply(rcode=3)
            if qtype == 1 and good_a:
                return Reply(addresses=(IPV4,))
            if qtype == 28 and good_aaaa:
                return Reply(addresses=("2001:db8:ffff::10" if higher_block else IPV6,))
            return Reply(rcode=2)

        with DnsFixture(answer) as fixture, marker_servers() as received:
            with engine_process(
                binary, "sing-box", native_config(), directory
            ) as process:
                wait_listener(process, 10808)
                marker, error = connect_marker("partial.test")
                running = process.poll() is None
            reaped = process.returncode is not None
            records = list(fixture.records)
            destinations = list(received)
        counts = Counter(record["qtype"] for record in records)
        expected_marker = (good_a or good_aaaa) and not higher_block
        partial = good_a != good_aaaa
        observed = (
            marker == expected_marker
            and bool(destinations) == expected_marker
            and counts[1] > 0
            and counts[28] > 0
            and running
            and reaped
        )
        case = {
            "case": label,
            "marker_received": marker,
            "marker_destinations": destinations,
            "dns_counts": {"A": counts[1], "AAAA": counts[28]},
            "dns_records": records,
            "engine_alive_after_probe": running,
            "engine_reaped": reaped,
            "socks_error": error,
            "expected_under_strict_dual_stack_contract": "block"
            if not (good_a and good_aaaa) or higher_block
            else "connect",
            "contract_mismatch": partial and marker,
            "observation_matches_reproduction": observed,
        }
        (directory / "result.json").write_text(
            json.dumps(case, indent=2, ensure_ascii=False) + "\n"
        )
        cases.append(case)
    reproduced = all(case["observation_matches_reproduction"] for case in cases)
    result = {
        "engine": "sing-box 1.14.4",
        "gate": "G-DNS D05",
        "gate_supported": False,
        "result": "mismatch_reproduced" if reproduced else "probe_failed",
        "reproduction_passed": reproduced,
        "netns_isolated": True,
        "external_network": False,
        "source_refs": SOURCE_REFS,
        "cases": cases,
    }
    (output / "result.json").write_text(
        json.dumps(result, indent=2, ensure_ascii=False) + "\n"
    )
    print(json.dumps(result, ensure_ascii=False), flush=True)
    if not reproduced:
        raise RuntimeError("Матрица partial A/AAAA не подтвердила ожидаемые наблюдения")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--worker", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    binary = validated_binary(args.binary, "sing-box")
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
        timeout=60,
    )


if __name__ == "__main__":
    install_signal_handlers()
    main()
