"""TCP/UDP IPv4/IPv6 через TUN в изолированном Linux namespace."""

from __future__ import annotations

import argparse
import json
import logging
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

from engine_smoke import FIXTURES, install_signal_handlers, links, run_sandbox
from traffic_echo import PAYLOAD_SIZE, RESPONSE_PREFIX, echo_servers, receive_exact

LOG = logging.getLogger("yonik.traffic")
TARGETS = [(socket.AF_INET, "198.18.0.2"), (socket.AF_INET6, "2001:db8:746f::2")]


def ip(*arguments: str) -> str:
    return subprocess.run(
        ["ip", *arguments], check=True, capture_output=True, text=True, timeout=5
    ).stdout


def probe(family: int, address: str, kind: int) -> None:
    payload = os.urandom(PAYLOAD_SIZE)
    with socket.socket(family, kind) as client:
        client.settimeout(2)
        client.connect((address, 40000))
        client.sendall(payload)
        expected = RESPONSE_PREFIX + payload
        response = (
            receive_exact(client, len(expected))
            if kind == socket.SOCK_STREAM
            else client.recv(4096)
        )
        if response != expected:
            raise RuntimeError("Ответ echo не соответствует отправленной нагрузке")


def counters() -> tuple[int, int]:
    stats = json.loads(ip("-j", "-s", "link", "show", "dev", "yonik-test"))[0][
        "stats64"
    ]
    return stats["rx"]["packets"], stats["tx"]["packets"]


def worker(binary: Path, engine: str, host_namespace: int) -> None:
    if os.stat("/proc/self/ns/net").st_ino == host_namespace or links() != {"lo"}:
        raise RuntimeError("Требуется новый изолированный network namespace")
    ip("link", "set", "lo", "up")
    with echo_servers():
        for crash in (False, True):
            config = FIXTURES / f"{engine}-traffic.json"
            with subprocess.Popen(
                [str(binary), "run", "-c", str(config)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
            ) as process:
                try:
                    deadline = time.monotonic() + 5
                    while "yonik-test" not in links():
                        if process.poll() is not None:
                            raise RuntimeError("Ядро завершилось до создания TUN")
                        if time.monotonic() >= deadline:
                            raise TimeoutError("TUN не создан за 5 секунд")
                        time.sleep(0.05)
                    ip("address", "replace", "198.18.0.1/30", "dev", "yonik-test")
                    ip(
                        "-6",
                        "address",
                        "replace",
                        "2001:db8:746f::1/126",
                        "dev",
                        "yonik-test",
                        "nodad",
                    )
                    ip("link", "set", "yonik-test", "up")
                    for family, address in TARGETS:
                        routes = json.loads(
                            ip(
                                "-j",
                                "-6" if family == socket.AF_INET6 else "-4",
                                "route",
                                "get",
                                address,
                            )
                        )
                        if routes[0].get("dev") != "yonik-test":
                            raise RuntimeError("Тестовый маршрут обходит TUN")
                        for kind in (socket.SOCK_STREAM, socket.SOCK_DGRAM):
                            before = counters()
                            probe(family, address, kind)
                            after = counters()
                            if not all(
                                end > start
                                for start, end in zip(before, after, strict=True)
                            ):
                                raise RuntimeError(
                                    "Не подтверждён обмен в обе стороны TUN"
                                )
                            LOG.info(
                                "%s: %s/%s echo через TUN",
                                engine,
                                "IPv6" if family == socket.AF_INET6 else "IPv4",
                                "TCP" if kind == socket.SOCK_STREAM else "UDP",
                            )
                    if crash:
                        process.kill()
                    else:
                        process.terminate()
                    process.wait(timeout=5)
                    if links() != {"lo"}:
                        raise RuntimeError("После завершения остался TUN")
                    for family, address in TARGETS:
                        for kind in (socket.SOCK_STREAM, socket.SOCK_DGRAM):
                            try:
                                probe(family, address, kind)
                            except OSError:
                                continue
                            raise RuntimeError("Отрицательный контроль прошёл без ядра")
                    LOG.info(
                        "%s: cleanup и отрицательный контроль после %s",
                        engine,
                        "SIGKILL" if crash else "SIGTERM",
                    )
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=5)
                    if process.stderr is not None:
                        diagnostics = process.stderr.read(65536).decode(
                            errors="replace"
                        )
                        if diagnostics:
                            LOG.debug("%s", diagnostics)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine", choices=("xray", "sing-box"))
    parser.add_argument("binary", type=Path)
    parser.add_argument("--worker-namespace", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if args.worker_namespace is not None:
        worker(binary, args.engine, args.worker_namespace)
        return 0
    config = FIXTURES / f"{args.engine}-traffic.json"
    check = (
        [str(binary), "run", "-test", "-c", str(config)]
        if args.engine == "xray"
        else [str(binary), "check", "-c", str(config)]
    )
    subprocess.run(check, check=True, capture_output=True, timeout=15)
    run_sandbox(
        [
            "unshare",
            "--user",
            "--map-root-user",
            "--net",
            sys.executable,
            str(Path(__file__).resolve()),
            args.engine,
            str(binary),
            "--worker-namespace",
            str(os.stat("/proc/self/ns/net").st_ino),
        ],
        timeout=45,
    )
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        LOG.error("Проверка не пройдена: %s", error)
        raise SystemExit(1) from None
