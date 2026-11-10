"""Проверка присоединения закреплённого ядра к persistent TUN helper в изоляции."""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

from engine_smoke import FIXTURES, install_signal_handlers, run_sandbox
from engine_traffic import TARGETS, probe
from network_topology_smoke import bounded_run
from traffic_echo import echo_servers

LOG = logging.getLogger("yonik.owned-tun")
NAME = "tofu00000001"
_WORKER_BOOTSTRAP = """
import json, logging, runpy, sys
from pathlib import Path
sys.path.insert(0, str(Path(sys.argv[1]).parent))
module = runpy.run_path(sys.argv[1], run_name="yonik_owned_tun_worker")
logging.basicConfig(level=logging.INFO, format="%(message)s")
module["install_signal_handlers"]()
raise SystemExit(module["worker"](Path(sys.argv[2]), sys.argv[3], tuple(json.loads(sys.argv[4]))))
"""


def namespace() -> tuple[int, int]:
    value = os.stat("/proc/self/ns/net")
    return value.st_dev, value.st_ino


def ip(*arguments: str) -> list[dict[str, Any]]:
    code, output = bounded_run(["/usr/bin/ip", "-j", *arguments], timeout=5)
    if code:
        raise RuntimeError("ip отклонил синтетическую операцию")
    return json.loads(output) if output else []


def guard_namespace(host: tuple[int, int]) -> tuple[int, int]:
    current = namespace()
    if current[0] != host[0] or current[1] == host[1]:
        raise RuntimeError("Требуется отдельный network namespace")
    if {item["ifname"] for item in ip("link", "show")} != {"lo"}:
        raise RuntimeError("Требуется пустой network namespace")
    return current


def identity() -> tuple[int, str]:
    value = ip("link", "show", "dev", NAME)
    if len(value) != 1 or value[0]["ifname"] != NAME:
        raise RuntimeError("Неоднозначный интерфейс fixture")
    return value[0]["ifindex"], value[0].get("ifalias", "")


def counters() -> tuple[int, int]:
    stats = ip("-s", "link", "show", "dev", NAME)[0]["stats64"]
    return stats["rx"]["packets"], stats["tx"]["packets"]


def config_for(engine: str) -> dict[str, Any]:
    value = json.loads((FIXTURES / f"{engine}-traffic.json").read_text())
    inbound = value["inbounds"][0]
    if engine == "xray":
        inbound["settings"]["name"] = NAME
    else:
        inbound["interface_name"] = NAME
    return value


def one_case(
    binary: Path, engine: str, multi_queue: bool, crash: bool
) -> dict[str, Any]:
    result: dict[str, Any] = {
        "multi_queue": multi_queue,
        "signal": "SIGKILL" if crash else "SIGTERM",
    }
    arguments = ["tuntap", "add", "dev", NAME, "mode", "tun"]
    if multi_queue:
        arguments.append("multi_queue")
    ip(*arguments)
    device, inode = namespace()
    alias = f"yonik:1:00000000:0000000000000007:0000000000000009:{device:016x}:{inode:016x}:0000000000000001:session"
    ip("link", "set", "dev", NAME, "alias", alias)
    original = identity()
    result["identity_before"] = {"ifindex": original[0], "alias": original[1]}
    ip("address", "add", "198.18.0.1/30", "dev", NAME)
    ip("-6", "address", "add", "2001:db8:746f::1/126", "dev", NAME, "nodad")
    ip("link", "set", "dev", NAME, "up")
    process: subprocess.Popen[bytes] | None = None
    try:
        with tempfile.TemporaryDirectory(prefix="yonik-owned-tun-") as directory:
            config = Path(directory) / "config.json"
            config.write_text(json.dumps(config_for(engine)))
            with tempfile.TemporaryFile() as diagnostics:
                process = subprocess.Popen(
                    [str(binary), "run", "-c", str(config)],
                    stdout=diagnostics,
                    stderr=subprocess.STDOUT,
                )
                deadline = time.monotonic() + 5
                while True:
                    if process.poll() is not None:
                        diagnostics.seek(0)
                        result.update(
                            result="INCOMPATIBLE",
                            exit=process.returncode,
                            diagnostics=diagnostics.read(65536).decode(
                                errors="replace"
                            ),
                        )
                        return result
                    try:
                        probe(socket.AF_INET, "198.18.0.2", socket.SOCK_STREAM)
                        break
                    except OSError:
                        if time.monotonic() >= deadline:
                            result.update(result="NOT_READY")
                            return result
                if identity() != original:
                    raise RuntimeError("Ядро изменило ifindex или alias helper")
                checks = []
                for family, address in TARGETS:
                    route = ip(
                        "-6" if family == socket.AF_INET6 else "-4",
                        "route",
                        "get",
                        address,
                    )
                    if len(route) != 1 or route[0].get("dev") != NAME:
                        raise RuntimeError("Маршрут обходит persistent TUN")
                    for kind in (socket.SOCK_STREAM, socket.SOCK_DGRAM):
                        before = counters()
                        probe(family, address, kind)
                        after = counters()
                        if not all(
                            end > start
                            for start, end in zip(before, after, strict=True)
                        ):
                            raise RuntimeError("Не подтверждён двусторонний обмен TUN")
                        checks.append(
                            f"{'IPv6' if family == socket.AF_INET6 else 'IPv4'}/{'TCP' if kind == socket.SOCK_STREAM else 'UDP'}"
                        )
                process.send_signal(signal.SIGKILL if crash else signal.SIGTERM)
                process.wait(timeout=5)
                remaining = ip("link", "show")
                same = any(
                    item.get("ifname") == NAME
                    and (item["ifindex"], item.get("ifalias", "")) == original
                    for item in remaining
                )
                for family, address in TARGETS:
                    for kind in (socket.SOCK_STREAM, socket.SOCK_DGRAM):
                        try:
                            probe(family, address, kind)
                        except OSError:
                            continue
                        raise RuntimeError("Отрицательный контроль прошёл без ядра")
                result.update(
                    result="PASS" if same else "ENGINE_REMOVED_HELPER_TUN",
                    traffic=checks,
                    identity_preserved_after_exit=same,
                    negative_control=True,
                )
                return result
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        # Единственная синтетическая netns; имя и alias проверены до удаления.
        remaining = [item for item in ip("link", "show") if item.get("ifname") == NAME]
        if remaining:
            if identity() != original:
                raise RuntimeError("Cleanup не присваивает изменённый интерфейс")
            ip("link", "delete", "dev", NAME)
        if {item["ifname"] for item in ip("link", "show")} != {"lo"}:
            raise RuntimeError("Fixture оставила интерфейсы")


def supported_modes(results: list[dict[str, Any]]) -> list[str]:
    return [
        "multi_queue" if multi_queue else "single_queue"
        for multi_queue in (False, True)
        if all(
            any(
                case.get("multi_queue") == multi_queue
                and case.get("signal") == signal_name
                and case.get("result") == "PASS"
                for case in results
            )
            for signal_name in ("SIGTERM", "SIGKILL")
        )
    ]


def worker(binary: Path, engine: str, host: tuple[int, int]) -> int:
    guard_namespace(host)
    for target in ("/tmp", "/run"):
        subprocess.run(
            ["/usr/bin/mount", "-t", "tmpfs", "-o", "size=16M", "tmpfs", target],
            check=True,
            timeout=5,
        )
    ip("link", "set", "lo", "up")
    results = []
    with echo_servers():
        for multi_queue in (False, True):
            for crash in (False, True):
                results.append(one_case(binary, engine, multi_queue, crash))
    modes = supported_modes(results)
    with binary.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    print(
        json.dumps(
            {
                "engine": engine,
                "sha256": digest,
                "supported_modes": modes,
                "route_ownership_verified": False,
                "cases": results,
                "cleanup": "PASS",
            },
            ensure_ascii=False,
            indent=2,
        )
    )
    # Helper создаёт single_queue: другой рабочий режим не закрывает этот gate.
    return 0 if "single_queue" in modes else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine", choices=("xray", "sing-box"))
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    run_sandbox(
        [
            "/usr/bin/unshare",
            "--user",
            "--map-root-user",
            "--net",
            "--mount",
            "--pid",
            "--fork",
            "--mount-proc",
            sys.executable,
            "-c",
            _WORKER_BOOTSTRAP,
            str(Path(__file__).resolve()),
            str(binary),
            args.engine,
            json.dumps(namespace()),
        ],
        timeout=60,
    )
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        LOG.error("Проверка не выполнена: %s", error)
        raise SystemExit(1) from None
