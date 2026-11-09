"""Ограниченные DNS fixtures для синтетических native gate в отдельном netns."""

from __future__ import annotations

import hashlib
import ipaddress
import json
import os
import socket
import struct
import subprocess
import threading
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any

DIGESTS = {
    "xray": "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1",
    "sing-box": "1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a",
}
MAX_PACKET = 65535
MAX_RECORDS = 128


@dataclass(frozen=True)
class Reply:
    addresses: tuple[str, ...] = ()
    rcode: int = 0
    ttl: int = 0
    drop: bool = False


def decode_name(packet: bytes, offset: int) -> tuple[str, int]:
    labels = []
    visited: set[int] = set()
    resume = None
    jumps = 0
    for _ in range(256):
        if offset >= len(packet) or offset in visited:
            raise ValueError("Некорректное DNS name/pointer")
        visited.add(offset)
        length = packet[offset]
        if length & 0xC0 == 0xC0:
            jumps += 1
            if offset + 1 >= len(packet) or jumps > 16:
                raise ValueError("Некорректный DNS pointer")
            if resume is None:
                resume = offset + 2
            offset = ((length & 0x3F) << 8) | packet[offset + 1]
        elif length == 0:
            name = ".".join(labels)
            if len(name) > 253:
                raise ValueError("Слишком длинное DNS name")
            return name, resume if resume is not None else offset + 1
        elif length > 63 or offset + 1 + length > len(packet):
            raise ValueError("Некорректный DNS label")
        else:
            labels.append(packet[offset + 1 : offset + 1 + length].decode("ascii"))
            offset += length + 1
    raise ValueError("Превышена вложенность DNS name")


def question_bytes(name: str, qtype: int) -> bytes:
    labels = name.encode("ascii").split(b".")
    if (
        not name
        or len(name) > 253
        or any(not label or len(label) > 63 for label in labels)
    ):
        raise ValueError("Некорректное DNS name")
    return (
        b"".join(bytes([len(label)]) + label for label in labels)
        + b"\0"
        + struct.pack("!HH", qtype, 1)
    )


def read_question(packet: bytes) -> tuple[str, int, int]:
    if (
        len(packet) < 12
        or len(packet) > MAX_PACKET
        or struct.unpack_from("!H", packet, 4)[0] != 1
    ):
        raise ValueError("Ожидается один DNS question")
    name, offset = decode_name(packet, 12)
    if offset + 4 > len(packet):
        raise ValueError("Обрезан DNS question")
    qtype, qclass = struct.unpack_from("!HH", packet, offset)
    if qclass != 1:
        raise ValueError("Ожидается DNS IN")
    return name, qtype, offset + 4


def make_response(packet: bytes, reply: Reply) -> bytes:
    name, qtype, _ = read_question(packet)
    if (
        len(reply.addresses) > 64
        or not 0 <= reply.rcode <= 15
        or not 0 <= reply.ttl <= 0xFFFFFFFF
    ):
        raise ValueError("Некорректный DNS fixture reply")
    answers = []
    for text in reply.addresses:
        address = ipaddress.ip_address(text)
        if qtype not in (1, 28) or address.version != (4 if qtype == 1 else 6):
            raise ValueError("DNS fixture family mismatch")
        answers.append(
            b"\xc0\x0c"
            + struct.pack("!HHIH", qtype, 1, reply.ttl, len(address.packed))
            + address.packed
        )
    flags = 0x8080 | (struct.unpack_from("!H", packet, 2)[0] & 0x0100) | reply.rcode
    return (
        packet[:2]
        + struct.pack("!HHHHH", flags, 1, len(answers), 0, 0)
        + question_bytes(name, qtype)
        + b"".join(answers)
    )


def response_addresses(
    packet: bytes,
    identifier: bytes,
    name: str,
    qtype: int,
    ttl_capture: list[int] | None = None,
) -> list[str]:
    actual_name, actual_type, offset = read_question(packet)
    flags, _, count, authority, additional = struct.unpack_from("!HHHHH", packet, 2)
    if packet[:2] != identifier or not flags & 0x8000 or flags & 0x020F:
        raise ValueError("DNS id/error/truncation mismatch")
    if (
        actual_name.lower() != name.lower()
        or actual_type != qtype
        or count > 64
        or count + authority + additional > MAX_RECORDS
    ):
        raise ValueError("DNS question/count mismatch")
    addresses = []
    for index in range(count + authority + additional):
        owner, offset = decode_name(packet, offset)
        if offset + 10 > len(packet):
            raise ValueError("Обрезан DNS resource header")
        kind, qclass, ttl, size = struct.unpack_from("!HHIH", packet, offset)
        offset += 10
        if offset + size > len(packet):
            raise ValueError("Обрезан DNS resource payload")
        if index < count and kind == qtype and qclass == 1:
            if owner.lower() != name.lower() or size != (4 if qtype == 1 else 16):
                raise ValueError("DNS answer owner/family mismatch")
            addresses.append(str(ipaddress.ip_address(packet[offset : offset + size])))
            if ttl_capture is not None:
                ttl_capture.append(ttl)
        offset += size
    if offset != len(packet):
        raise ValueError("Лишние данные DNS reply")
    return addresses


def dns_query(
    name: str,
    qtype: int = 1,
    port: int = 1053,
    timeout: float = 2,
    ttl_capture: list[int] | None = None,
) -> list[str]:
    identifier = os.urandom(2)
    packet = (
        identifier
        + struct.pack("!HHHHH", 0x0100, 1, 0, 0, 0)
        + question_bytes(name, qtype)
    )
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
        client.settimeout(timeout)
        client.connect(("127.0.0.1", port))
        client.send(packet)
        return response_addresses(
            client.recv(MAX_PACKET), identifier, name, qtype, ttl_capture
        )


class DnsFixture:
    def __init__(
        self, handler: Callable[[str, int, int], Reply], port: int = 1053
    ) -> None:
        self.handler = handler
        self.records: list[dict[str, Any]] = []
        self._errors: list[str] = []
        self._stop = threading.Event()
        self._socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self._socket.bind(("127.0.0.1", port))
        self._socket.settimeout(0.1)
        self.port = self._socket.getsockname()[1]
        self._thread = threading.Thread(target=self._serve)

    def _serve(self) -> None:
        while not self._stop.is_set():
            try:
                packet, peer = self._socket.recvfrom(4096)
            except socket.timeout:
                continue
            except OSError:
                if not self._stop.is_set():
                    self._errors.append("DNS socket failure")
                break
            try:
                if (
                    len(self.records) >= 512
                    or struct.unpack_from("!H", packet, 2)[0] & 0x8000
                ):
                    raise ValueError("DNS fixture query limit/type")
                name, qtype, _ = read_question(packet)
                reply = self.handler(name, qtype, len(self.records) + 1)
                self.records.append(
                    {
                        "name": name,
                        "qtype": qtype,
                        "rcode": reply.rcode,
                        "addresses": list(reply.addresses),
                        "drop": reply.drop,
                    }
                )
                if not reply.drop:
                    self._socket.sendto(make_response(packet, reply), peer)
            except (ValueError, OSError, struct.error) as error:
                self._errors.append(type(error).__name__)
                break

    def __enter__(self) -> DnsFixture:
        self._thread.start()
        return self

    def __exit__(self, *_: object) -> None:
        self._stop.set()
        self._socket.close()
        self._thread.join(timeout=2)
        if self._thread.is_alive() or self._errors:
            raise RuntimeError("DNS fixture cleanup/protocol failure")


def verify_namespace(host_inode: int) -> None:
    if os.stat("/proc/self/ns/net").st_ino == host_inode:
        raise RuntimeError("Нет изоляции network namespace")
    links = json.loads(subprocess.check_output(["ip", "-j", "link", "show"], timeout=3))
    if {link["ifname"] for link in links} != {"lo"}:
        raise RuntimeError("Namespace содержит чужие интерфейсы")
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True, timeout=3)


def validated_binary(path: Path, engine: str) -> Path:
    path = path.resolve(strict=True)
    with path.open("rb") as source:
        if hashlib.file_digest(source, "sha256").hexdigest() != DIGESTS[engine]:
            raise RuntimeError("Неверный digest закреплённого ядра")
    return path


@contextmanager
def engine_process(
    binary: Path, engine: str, config: dict[str, Any], workspace: Path
) -> Iterator[subprocess.Popen[bytes]]:
    workspace.mkdir(parents=True, exist_ok=True)
    config_path = workspace / f"{engine}.json"
    descriptor = os.open(config_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(descriptor, "w") as target:
        json.dump(config, target)
    with (workspace / f"{engine}.log").open("wb") as log:
        check = (
            [str(binary), "run", "-test", "-c", str(config_path)]
            if engine == "xray"
            else [str(binary), "check", "-c", str(config_path)]
        )
        command = [str(binary), "run", "-c", str(config_path)]
        subprocess.run(check, check=True, stdout=log, stderr=log, timeout=8)
        child = subprocess.Popen(command, stdout=log, stderr=log)
        try:
            yield child
        finally:
            if child.poll() is None:
                child.terminate()
            try:
                child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=3)
