"""Изолированная сетевая fixture для явно запущенного ignored Rust-теста."""

from __future__ import annotations

import argparse
import ipaddress
import json
import os
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

PEER = "yonik-peer"
PORTS = (24081, 24082)
PAYLOAD = b"yonik-public-block-test"
TIMEOUT = 0.3
IP_PKTINFO = 8  # Linux uapi: Python не экспортирует константу во всех сборках.


def guard_namespace() -> None:
    """Откажись от любых сетевых операций без подтверждённой изоляции."""
    expected = os.environ.get("YONIK_NFT_TEST_HOST_NETNS", "")
    parts = expected.split(":")
    if len(parts) != 2 or not all(part.isdecimal() for part in parts):
        raise RuntimeError("Не задана идентичность host netns")
    current = os.stat("/proc/self/ns/net")
    if (current.st_dev, current.st_ino) == tuple(map(int, parts)):
        raise RuntimeError("Операции в host netns запрещены")


def run_ip(*arguments: str) -> str:
    result = subprocess.run(
        ["/usr/bin/ip", *arguments],
        check=True,
        capture_output=True,
        text=True,
        timeout=5,
    )
    return result.stdout


def setup() -> dict[str, int]:
    if set(name for _, name in socket.if_nameindex()) != {"lo"}:
        raise RuntimeError("Setup требует пустой netns с единственным lo")
    # /run предоставляет runner: ip netns не должен использовать host bind mounts.
    mountinfo = Path("/proc/self/mountinfo").read_text()
    if not any(
        line.split()[4] == "/run" and " - tmpfs " in line
        for line in mountinfo.splitlines()
    ):
        raise RuntimeError("Setup требует отдельный tmpfs в /run")
    run_ip("netns", "add", PEER)
    run_ip("link", "set", "lo", "up")
    run_ip("-n", PEER, "link", "set", "lo", "up")
    for local, remote in (("uplink", "uplink-peer"), ("vpn", "vpn-peer")):
        run_ip("link", "add", local, "type", "veth", "peer", "name", remote)
        run_ip("link", "set", remote, "netns", PEER)
        run_ip("link", "set", local, "up")
        run_ip("-n", PEER, "link", "set", remote, "up")
    # Автоматический DAD делает cold-neighbour контроль зависимым от времени.
    # В изолированной fixture адреса известны и уникальны: задаём их с nodad.
    for prefix, interface, address in (
        ((), "uplink", "fe80::1/64"),
        ((), "vpn", "fe80::3/64"),
        (("-n", PEER), "uplink-peer", "fe80::2/64"),
        (("-n", PEER), "vpn-peer", "fe80::4/64"),
    ):
        run_ip(*prefix, "-6", "address", "flush", "dev", interface, "scope", "link")
        run_ip(*prefix, "-6", "address", "add", address, "dev", interface, "nodad")
    for interface, addresses in (
        ("uplink", ("198.18.1.1/24", "2001:db8:1::1/64")),
        ("vpn", ("10.42.1.1/24", "fd42:1::1/64")),
    ):
        for address in addresses:
            run_ip("address", "add", address, "dev", interface, "nodad")
    for interface, addresses in (
        (
            "uplink-peer",
            (
                "198.18.1.2/24",
                "198.18.1.3/24",
                "2001:db8:1::2/64",
                "2001:db8:1::3/64",
                "10.42.0.2/16",
                "fd42::2/48",
            ),
        ),
        ("vpn-peer", ("10.42.1.2/24", "fd42:1::2/64")),
    ):
        for address in addresses:
            run_ip("-n", PEER, "address", "add", address, "dev", interface, "nodad")
    run_ip("route", "add", "10.42.0.0/16", "dev", "uplink")
    run_ip("-6", "route", "add", "fd42::/48", "dev", "uplink")
    return {name: socket.if_nametoindex(name) for name in ("uplink", "vpn", "lo")}


def reroute_private() -> dict[str, object]:
    # NDP отвечает только на адрес своего интерфейса; перенос сохраняет
    # доступность endpoint после смены маршрута и исключает ложный DROP.
    run_ip("-n", PEER, "-6", "address", "del", "fd42:1::2/64", "dev", "vpn-peer")
    run_ip(
        "-n",
        PEER,
        "-6",
        "address",
        "add",
        "fd42:1::2/128",
        "dev",
        "uplink-peer",
        "nodad",
    )
    run_ip("route", "replace", "10.42.1.0/24", "dev", "uplink", "src", "198.18.1.1")
    run_ip(
        "-6",
        "route",
        "replace",
        "fd42:1::/64",
        "dev",
        "uplink",
        "src",
        "2001:db8:1::1",
        "metric",
        "256",
    )
    return {
        "ipv4": json.loads(run_ip("-j", "route", "get", "10.42.1.2")),
        "ipv6": json.loads(run_ip("-6", "-j", "route", "get", "fd42:1::2")),
    }


def cold_neighbors() -> dict[str, bool]:
    for local, remote in (("uplink", "uplink-peer"), ("vpn", "vpn-peer")):
        run_ip("-6", "neigh", "flush", "dev", local)
        run_ip("-n", PEER, "-6", "neigh", "flush", "dev", remote)
    return {"flushed": True}


def nat_redirect() -> dict[str, bool]:
    commands = []
    for family, source, destination in (
        ("ip", "10.42.0.2", "198.18.1.2"),
        ("ip6", "fd42::2", "2001:db8:1::2"),
    ):
        table = f"yonik-fixture-nat-{family}"
        commands.extend(
            [
                {"create": {"table": {"family": family, "name": table}}},
                {
                    "add": {
                        "chain": {
                            "family": family,
                            "table": table,
                            "name": "output",
                            "type": "nat",
                            "hook": "output",
                            "prio": -100,
                            "policy": "accept",
                        }
                    }
                },
                {
                    "add": {
                        "rule": {
                            "family": family,
                            "table": table,
                            "chain": "output",
                            "expr": [
                                {
                                    "match": {
                                        "op": "==",
                                        "left": {
                                            "payload": {
                                                "protocol": family,
                                                "field": "daddr",
                                            }
                                        },
                                        "right": source,
                                    }
                                },
                                {
                                    "match": {
                                        "op": "==",
                                        "left": {
                                            "payload": {
                                                "protocol": "tcp",
                                                "field": "dport",
                                            }
                                        },
                                        "right": 24082,
                                    }
                                },
                                {"dnat": {"addr": destination, "port": 24082}},
                            ],
                        }
                    }
                },
            ]
        )
    subprocess.run(
        ["/usr/bin/nft", "-j", "-f", "-"],
        input=json.dumps({"nftables": commands}),
        capture_output=True,
        text=True,
        timeout=5,
        check=True,
    )
    return {"installed": True}


def neighbor_control() -> dict[str, object]:
    addresses = json.loads(run_ip("-j", "-6", "address", "show", "dev", "uplink"))
    address = next(
        info["local"] for info in addresses[0]["addr_info"] if info["scope"] == "link"
    )
    run_ip("-n", PEER, "-6", "neigh", "flush", "dev", "uplink-peer")
    # Echo reply не разрешён политикой; проверяем именно завершение NDP/NA,
    # а не доступность ICMP echo приложения через публичную блокировку.
    ping = subprocess.run(
        [
            "/usr/bin/ip",
            "netns",
            "exec",
            PEER,
            "/usr/bin/ping",
            "-6",
            "-I",
            "uplink-peer",
            "-c",
            "1",
            "-W",
            "1",
            address,
        ],
        capture_output=True,
        timeout=3,
        check=False,
    )
    neighbors = json.loads(
        run_ip("-n", PEER, "-j", "-6", "neigh", "show", "dev", "uplink-peer")
    )
    resolved = any(
        item.get("dst") == address
        and item.get("lladdr")
        and not {"INCOMPLETE", "FAILED"}.intersection(item.get("state", []))
        for item in neighbors
    )
    return {
        "resolved": resolved,
        "neighbors": neighbors,
        "ping_exit": ping.returncode,
        "ping_error": ping.stderr.decode(errors="replace"),
    }


def serve(endpoint: socket.socket, stop: threading.Event, tcp: bool) -> None:
    while not stop.is_set():
        try:
            if tcp:
                connection, _ = endpoint.accept()
                with connection:
                    connection.settimeout(TIMEOUT)
                    while not stop.is_set():
                        data = connection.recv(4096)
                        if not data:
                            break
                        connection.sendall(data)
            else:
                data, metadata, _, address = endpoint.recvmsg(4096, 256)
                # Wildcard socket должен ответить с адреса назначения запроса.
                endpoint.sendmsg([data], metadata, 0, address)
        except (TimeoutError, OSError):
            # Клиент может исчезнуть при проверке DROP или завершении процесса.
            continue


def peer() -> None:
    endpoints: list[socket.socket] = []
    workers: list[threading.Thread] = []
    stop = threading.Event()
    try:
        for port in PORTS:
            for family, address in (
                (socket.AF_INET, "0.0.0.0"),
                (socket.AF_INET6, "::"),
            ):
                for kind in (socket.SOCK_STREAM, socket.SOCK_DGRAM):
                    endpoint = socket.socket(family, kind)
                    endpoints.append(endpoint)
                    endpoint.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                    if family == socket.AF_INET6:
                        endpoint.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
                    if kind == socket.SOCK_DGRAM:
                        if family == socket.AF_INET:
                            endpoint.setsockopt(socket.IPPROTO_IP, IP_PKTINFO, 1)
                        else:
                            endpoint.setsockopt(
                                socket.IPPROTO_IPV6, socket.IPV6_RECVPKTINFO, 1
                            )
                    endpoint.settimeout(TIMEOUT)
                    endpoint.bind((address, port))
                    if kind == socket.SOCK_STREAM:
                        endpoint.listen(16)
                    worker = threading.Thread(
                        target=serve,
                        args=(endpoint, stop, kind == socket.SOCK_STREAM),
                    )
                    workers.append(worker)
                    worker.start()
        print("READY", flush=True)
        sys.stdin.buffer.read()
    finally:
        stop.set()
        for endpoint in endpoints:
            endpoint.close()
        for worker in workers:
            worker.join(timeout=1)


def probe(address: str, protocol: str, port: int, mark: int | None) -> bool:
    parsed = ipaddress.ip_address(address)
    family = socket.AF_INET if parsed.version == 4 else socket.AF_INET6
    kind = socket.SOCK_STREAM if protocol == "tcp" else socket.SOCK_DGRAM
    deadline = time.monotonic() + TIMEOUT
    try:
        with socket.socket(family, kind) as connection:
            if mark is not None:
                connection.setsockopt(
                    socket.SOL_SOCKET, socket.SO_MARK, mark.to_bytes(4, sys.byteorder)
                )
            connection.settimeout(max(0.001, deadline - time.monotonic()))
            connection.connect((str(parsed), port))
            connection.settimeout(max(0.001, deadline - time.monotonic()))
            connection.sendall(PAYLOAD)
            response = bytearray()
            while len(response) < len(PAYLOAD):
                connection.settimeout(max(0.001, deadline - time.monotonic()))
                chunk = connection.recv(len(PAYLOAD) - len(response))
                if not chunk:
                    return False
                response.extend(chunk)
                if kind == socket.SOCK_DGRAM:
                    break
            return response == PAYLOAD
    except OSError:
        return False


def bounded_integer(value: str, maximum: int) -> int:
    number = int(value, 0)
    if not 0 <= number <= maximum:
        raise argparse.ArgumentTypeError("Число вне разрешённого диапазона")
    return number


def port_number(value: str) -> int:
    number = bounded_integer(value, 65535)
    if number == 0:
        raise argparse.ArgumentTypeError("Нужен ненулевой порт")
    return number


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="mode", required=True)
    for mode in (
        "setup",
        "peer",
        "reroute-private",
        "cold-neighbors",
        "neighbor-control",
        "nat-redirect",
    ):
        commands.add_parser(mode)
    client = commands.add_parser("probe")
    client.add_argument("--address", required=True, type=ipaddress.ip_address)
    client.add_argument("--protocol", required=True, choices=("tcp", "udp"))
    client.add_argument("--port", type=port_number, default=24081)
    client.add_argument("--mark", type=lambda value: bounded_integer(value, 0xFFFFFFFF))
    args = parser.parse_args()
    guard_namespace()
    match args.mode:
        case "setup":
            indices = setup()
            indices["loopback"] = indices.pop("lo")
            print(json.dumps(indices))
        case "peer":
            peer()
        case "probe":
            print(
                json.dumps(
                    {
                        "reachable": probe(
                            str(args.address), args.protocol, args.port, args.mark
                        )
                    }
                )
            )
        case "reroute-private":
            print(json.dumps(reroute_private()))
        case "cold-neighbors":
            print(json.dumps(cold_neighbors()))
        case "neighbor-control":
            print(json.dumps(neighbor_control()))
        case "nat-redirect":
            print(json.dumps(nat_redirect()))


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        sys.exit(f"Ошибка fixture: {type(error).__name__}")
