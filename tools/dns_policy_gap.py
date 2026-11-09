"""Синтетическое доказательство Xray IPOnDemand DNS fail-open в новом netns."""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

from engine_smoke import install_signal_handlers, run_sandbox
from traffic_echo import receive_exact

LOG = logging.getLogger("yonik.dns-gap")
SHA = "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1"
MARKER = b"DNS_FAIL_OPEN_CONFIRMED"


def worker(host_ns: int, binary: Path, workspace: Path) -> None:
    if os.stat("/proc/self/ns/net").st_ino == host_ns:
        raise RuntimeError("Нет изоляции network namespace")
    links = json.loads(subprocess.check_output(["ip", "-j", "link", "show"], timeout=3))
    if {link["ifname"] for link in links} != {"lo"}:
        raise RuntimeError("Namespace содержит чужие интерфейсы")
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True, timeout=3)
    queries = [0]
    stop = threading.Event()
    with (
        socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as dns,
        socket.socket() as echo,
    ):
        dns.bind(("127.0.0.1", 1053))
        dns.settimeout(0.1)
        echo.bind(("127.0.0.1", 24081))
        echo.listen()
        echo.settimeout(0.1)

        def dns_loop() -> None:
            while not stop.is_set():
                try:
                    packet, peer = dns.recvfrom(4096)
                except socket.timeout:
                    continue
                if len(packet) < 12:
                    continue
                queries[0] += 1
                # QR=1, RD=1, RA=1, RCODE=SERVFAIL; сохраняем вопрос.
                reply = (
                    packet[:2] + struct.pack("!HHHHH", 0x8182, 1, 0, 0, 0) + packet[12:]
                )
                dns.sendto(reply, peer)

        def echo_loop() -> None:
            while not stop.is_set():
                try:
                    conn, _ = echo.accept()
                except socket.timeout:
                    continue
                with conn:
                    conn.sendall(MARKER)

        threads = [
            threading.Thread(target=dns_loop),
            threading.Thread(target=echo_loop),
        ]
        for thread in threads:
            thread.start()
        config = {
            "log": {"loglevel": "debug"},
            "dns": {
                "tag": "dns",
                "queryStrategy": "UseIPv4",
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
                    "settings": {"redirect": "127.0.0.1:24081"},
                },
            ],
            "routing": {
                "domainStrategy": "IPOnDemand",
                "rules": [
                    {
                        "type": "field",
                        "inboundTag": ["dns"],
                        "outboundTag": "dns-direct",
                    },
                    {
                        "type": "field",
                        "ip": ["198.51.100.0/24"],
                        "outboundTag": "block",
                    },
                    {
                        "type": "field",
                        "domain": ["full:probe.test"],
                        "outboundTag": "direct",
                    },
                ],
            },
        }
        try:
            with tempfile.TemporaryFile(dir=workspace) as log:
                config_path = workspace / "xray.json"
                config_path.write_text(json.dumps(config))
                config_path.chmod(0o600)
                subprocess.run(
                    [str(binary), "run", "-test", "-c", str(config_path)],
                    check=True,
                    stdout=log,
                    stderr=log,
                    timeout=5,
                )
                engine = subprocess.Popen(
                    [str(binary), "run", "-c", str(config_path)], stdout=log, stderr=log
                )
                try:
                    deadline = time.monotonic() + 5
                    while True:
                        try:
                            control = socket.create_connection(
                                ("127.0.0.1", 10808), timeout=0.2
                            )
                            break
                        except OSError:
                            if (
                                engine.poll() is not None
                                or time.monotonic() >= deadline
                            ):
                                raise RuntimeError("SOCKS listener не запустился")
                            time.sleep(0.05)
                    with control:
                        control.settimeout(10)
                        control.sendall(b"\x05\x01\x00")
                        if receive_exact(control, 2) != b"\x05\x00":
                            raise RuntimeError("SOCKS greeting")
                        name = b"probe.test"
                        control.sendall(
                            b"\x05\x01\x00\x03"
                            + bytes([len(name)])
                            + name
                            + struct.pack("!H", 443)
                        )
                        header = receive_exact(control, 4)
                        if header[:3] != b"\x05\x00\x00":
                            raise RuntimeError(f"SOCKS reply: {header.hex()}")
                        if header[3] == 1:
                            receive_exact(control, 6)
                        elif header[3] == 4:
                            receive_exact(control, 18)
                        else:
                            raise RuntimeError("SOCKS ATYP")
                        marker = receive_exact(control, len(MARKER))
                    if marker != MARKER or not queries[0]:
                        raise RuntimeError("Нет подтверждения fail-open")
                    print(
                        json.dumps(
                            {
                                "marker": marker.decode(),
                                "dns_queries": queries[0],
                                "dns_reply": "SERVFAIL",
                                "engine_running": engine.poll() is None,
                                "netns_isolated": True,
                                "probe_exit": 0,
                            }
                        ),
                        flush=True,
                    )
                finally:
                    if engine.poll() is None:
                        engine.terminate()
                    try:
                        engine.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        engine.kill()
                        engine.wait(timeout=3)
                    log.seek(0)
                    for line in log.read(65536).decode(errors="replace").splitlines():
                        if any(
                            marker in line
                            for marker in (
                                "rcode: 2",
                                "returning nil",
                                "taking detour [direct]",
                            )
                        ):
                            LOG.info("%s", line)
                    LOG.info("engine_exit=%s", engine.returncode)
        finally:
            stop.set()
            for thread in threads:
                thread.join(timeout=3)
                if thread.is_alive():
                    raise RuntimeError("Синтетический сервер не завершился")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--worker", type=int)
    parser.add_argument("--workspace", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    if hashlib.sha256(binary.read_bytes()).hexdigest() != SHA:
        raise RuntimeError("SHA-256 закреплённого Xray не совпадает")
    if args.worker is not None:
        if args.workspace is None:
            raise RuntimeError("Нет workspace синтетического стенда")
        worker(args.worker, binary, args.workspace)
        return
    with tempfile.TemporaryDirectory(prefix="yonik-dns-gap-") as workspace:
        run_sandbox(
            [
                "unshare",
                "--user",
                "--map-root-user",
                "--net",
                sys.executable,
                str(Path(__file__).resolve()),
                str(binary),
                "--worker",
                str(os.stat("/proc/self/ns/net").st_ino),
                "--workspace",
                workspace,
            ],
            timeout=30,
        )


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    main()
