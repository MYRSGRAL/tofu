"""Контрпример повторного назначения NativeFakeIP в изолированном netns."""

from __future__ import annotations

import argparse
import contextlib
import json
import logging
import os
import socket
import struct
import sys
import threading
import time
from pathlib import Path
from typing import Iterator

from dns_fixture import dns_query, engine_process, validated_binary, verify_namespace
from engine_compile import socks_reply, wait_listener
from engine_smoke import install_signal_handlers, run_sandbox
from traffic_echo import receive_exact

LIMIT = 32


@contextlib.contextmanager
def markers() -> Iterator[list[str]]:
    """Каждый домен получает отдельный локальный TCP-маркер."""
    stopped = threading.Event()
    hits: list[str] = []
    sockets: list[socket.socket] = []
    threads: list[threading.Thread] = []

    def serve(listener: socket.socket, marker: str) -> None:
        while not stopped.is_set():
            try:
                connection, _ = listener.accept()
            except socket.timeout:
                continue
            with connection:
                hits.append(marker)
                connection.sendall(marker.encode())

    try:
        for index in range(LIMIT + 1):
            listener = socket.socket()
            sockets.append(listener)
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(("127.0.0.1", 24100 + index))
            listener.listen()
            listener.settimeout(0.1)
            thread = threading.Thread(target=serve, args=(listener, f"M{index:02d}"))
            threads.append(thread)
            thread.start()
        yield hits
    finally:
        stopped.set()
        for thread in threads:
            thread.join(timeout=2)
        for listener in sockets:
            listener.close()
        if any(thread.is_alive() for thread in threads):
            raise RuntimeError("TCP fixture не завершился")


def config(engine: str) -> dict:
    names = [f"name-{index}.fakeip.test" for index in range(LIMIT + 1)]
    if engine == "xray":
        return {
            "log": {"loglevel": "debug"},
            "fakedns": [{"ipPool": "198.18.0.0/30", "poolSize": 2}],
            "dns": {"servers": ["fakedns"], "queryStrategy": "UseIPv4"},
            "inbounds": [
                {
                    "tag": "dns-in",
                    "listen": "127.0.0.1",
                    "port": 1053,
                    "protocol": "dokodemo-door",
                    "settings": {"address": "127.0.0.1", "port": 53, "network": "udp"},
                },
                {
                    "tag": "socks",
                    "listen": "127.0.0.1",
                    "port": 10808,
                    "protocol": "socks",
                    "settings": {"auth": "noauth"},
                    "sniffing": {
                        "enabled": True,
                        "destOverride": ["fakedns"],
                        "metadataOnly": True,
                    },
                },
            ],
            "outbounds": [
                {"tag": "block", "protocol": "blackhole"},
                {"tag": "dns", "protocol": "dns"},
                *[
                    {
                        "tag": f"m{i}",
                        "protocol": "freedom",
                        "settings": {"redirect": f"127.0.0.1:{24100 + i}"},
                    }
                    for i in range(LIMIT + 1)
                ],
            ],
            "routing": {
                "domainStrategy": "AsIs",
                "rules": [
                    {"type": "field", "inboundTag": ["dns-in"], "outboundTag": "dns"},
                    *[
                        {
                            "type": "field",
                            "domain": [f"full:{name}"],
                            "outboundTag": f"m{i}",
                        }
                        for i, name in enumerate(names)
                    ],
                ],
            },
        }
    return {
        "log": {"level": "debug"},
        "dns": {
            "servers": [
                {
                    "type": "udp",
                    "tag": "unused",
                    "server": "127.0.0.1",
                    "server_port": 9,
                },
                {"type": "fakeip", "tag": "fake", "inet4_range": "198.18.0.0/30"},
            ],
            "rules": [
                {"domain_suffix": ["fakeip.test"], "action": "route", "server": "fake"}
            ],
            "final": "unused",
        },
        "inbounds": [
            {
                "type": "direct",
                "tag": "dns-in",
                "listen": "127.0.0.1",
                "listen_port": 1053,
            },
            {
                "type": "socks",
                "tag": "socks",
                "listen": "127.0.0.1",
                "listen_port": 10808,
            },
        ],
        "outbounds": [{"type": "direct", "tag": "marker"}],
        "route": {
            "default_domain_resolver": "unused",
            "rules": [
                {"inbound": ["dns-in"], "action": "hijack-dns"},
                *[
                    {
                        "domain": [name],
                        "action": "route",
                        "outbound": "marker",
                        "override_address": "127.0.0.1",
                        "override_port": 24100 + i,
                    }
                    for i, name in enumerate(names)
                ],
                {"action": "reject"},
            ],
        },
    }


def connect(address: str) -> str | None:
    try:
        with socket.create_connection(("127.0.0.1", 10808), timeout=1) as control:
            control.settimeout(1)
            control.sendall(b"\x05\x01\x00")
            if receive_exact(control, 2) != b"\x05\x00":
                raise RuntimeError("SOCKS greeting отклонён")
            control.sendall(
                b"\x05\x01\x00\x01" + socket.inet_aton(address) + struct.pack("!H", 443)
            )
            socks_reply(control)
            control.sendall(b"synthetic-probe")
            return receive_exact(control, 3).decode("ascii")
    except (OSError, ConnectionError, EOFError):
        return None


def run_engine(binary: Path, engine: str, output: Path) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    transcript: list[dict] = []

    def query(index: int, epoch: int) -> str:
        name = f"name-{index}.fakeip.test"
        ttls: list[int] = []
        addresses = dns_query(name, qtype=1, port=1053, ttl_capture=ttls)
        if len(addresses) != 1 or not addresses[0].startswith("198.18.0."):
            raise RuntimeError(f"Неожиданный FakeIP ответ: {addresses}")
        if len(ttls) != 1:
            raise RuntimeError("Нет DNS TTL для FakeIP")
        transcript.append(
            {"epoch": epoch, "qname": name, "fake_ip": addresses[0], "ttl": ttls[0]}
        )
        return addresses[0]

    with markers() as hits:
        first = output / "session-1"
        first.mkdir(exist_ok=True)
        with engine_process(binary, engine, config(engine), first) as process:
            wait_listener(process, 10808)
            unknown = connect("198.18.0.3")
            if unknown is not None or hits:
                raise RuntimeError("Неизвестный FakeIP достиг маркера")
            old = query(0, 1)
            original_ttl = transcript[-1]["ttl"]
            original_at = time.monotonic()
            valid = connect(old)
            if valid != "M00":
                raise RuntimeError(f"Контроль A не достиг маркера A: {valid}")
            reused: dict | None = None
            for index in range(1, LIMIT + 1):
                assigned = query(index, 1)
                if assigned == old:
                    actual = connect(old)
                    if actual != f"M{index:02d}":
                        raise RuntimeError(f"Повторный FakeIP не достиг B: {actual}")
                    elapsed = time.monotonic() - original_at
                    if not 0 <= elapsed < original_ttl:
                        raise RuntimeError(
                            "Повторное назначение до исходного DNS TTL не подтверждено"
                        )
                    reused = {
                        "original_dns_ttl": original_ttl,
                        "elapsed_seconds": elapsed,
                        "within_original_ttl": True,
                        "old_ip": old,
                        "old_qname": "name-0.fakeip.test",
                        "new_qname": f"name-{index}.fakeip.test",
                        "initial_marker": valid,
                        "stale_ip_marker": actual,
                        "allocation_count": index + 1,
                    }
                    break
            if reused is None:
                raise RuntimeError("Повторное назначение не найдено за 32 запроса")
            first_alive = process.poll() is None
            if not first_alive:
                raise RuntimeError(
                    "Первое ядро неожиданно завершилось во время проверки"
                )
        first_reaped = process.returncode is not None
        second = output / "session-2"
        second.mkdir(exist_ok=True)
        with engine_process(binary, engine, config(engine), second) as process:
            wait_listener(process, 10808)
            before = len(hits)
            empty = connect(old)
            if empty is not None or len(hits) != before:
                raise RuntimeError("Старый FakeIP известен в пустой новой сессии")
            epoch_reuse: dict | None = None
            for index in range(1, LIMIT + 1):
                if query(index, 2) == old:
                    actual = connect(old)
                    if actual != f"M{index:02d}":
                        raise RuntimeError(
                            "Нет wire-proof повторного назначения после restart"
                        )
                    epoch_reuse = {
                        "old_ip": old,
                        "new_qname": f"name-{index}.fakeip.test",
                        "stale_ip_marker": actual,
                    }
                    break
            if epoch_reuse is None:
                raise RuntimeError(
                    "Нет повторного назначения после restart за 32 запроса"
                )
            second_alive = process.poll() is None
            if not second_alive:
                raise RuntimeError(
                    "Второе ядро неожиданно завершилось во время проверки"
                )
        second_reaped = process.returncode is not None
        if not first_reaped or not second_reaped:
            raise RuntimeError("Не подтверждён reap обоих экземпляров ядра")
    return {
        "engine": engine,
        "gate_supported": False,
        "netns_isolated": True,
        "unknown_fake_ip_marker": unknown,
        "empty_new_session_marker": empty,
        "same_session_stale_alias": reused,
        "new_session_stale_alias": epoch_reuse,
        "engines_alive_during_probe": first_alive and second_alive,
        "engines_reaped": first_reaped and second_reaped,
        "dns_wire_transcript": transcript,
        "scope": "Синтетический маленький pool; фактический TTL измерен, весь D07 не закрыт",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("xray", type=Path)
    parser.add_argument("sing_box", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--worker", type=int)
    args = parser.parse_args()
    binaries = {
        "xray": validated_binary(args.xray, "xray"),
        "sing-box": validated_binary(args.sing_box, "sing-box"),
    }
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if args.worker is not None:
        verify_namespace(args.worker)
        results = [
            run_engine(binary, engine, output / engine)
            for engine, binary in binaries.items()
        ]
        (output / "result.json").write_text(
            json.dumps(
                {"gate_supported": False, "results": results},
                ensure_ascii=False,
                indent=2,
            )
            + "\n"
        )
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
            str(binaries["xray"]),
            str(binaries["sing-box"]),
            "--output",
            str(output),
            "--worker",
            str(os.stat("/proc/self/ns/net").st_ino),
        ],
        timeout=90,
    )


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    main()
