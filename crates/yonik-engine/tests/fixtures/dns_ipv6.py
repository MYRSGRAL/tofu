"""Изолированный IPv6 gate; запускается только через native_dns_modes."""

import json
import os
import socket
import struct
import sys
import threading
import time
from collections.abc import Callable
from pathlib import Path

sys.path.insert(0, str(Path.cwd() / "tools"))
from dns_fixture import (
    Reply,
    dns_query,
    engine_process,
    make_response,
    question_bytes,
    read_question,
    response_addresses,
    validated_binary,
    verify_namespace,
)
from traffic_echo import receive_exact

base = Path(os.environ["YONIK_DNS_EVIDENCE"])
verify_namespace(int(sys.argv[1]))
stop = threading.Event()
seen = []
queries = []
errors = []


def upstream_response(packet: bytes) -> bytes:
    name, qtype, _ = read_question(packet)
    queries.append(dict(name=name, qtype=qtype, wire=packet.hex()))
    addresses = {1: ("192.0.2.20",), 28: ("2001:db8::20",)}.get(qtype, ())
    return make_response(packet, Reply(addresses=addresses, ttl=1))


def upstream() -> None:
    with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as s:
        s.bind(("::1", 1053))
        s.settimeout(0.2)
        while not stop.is_set():
            try:
                packet, peer = s.recvfrom(4096)
            except socket.timeout:
                continue
            s.sendto(upstream_response(packet), peer)


def proxy() -> None:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 24082))
        s.listen()
        s.settimeout(0.2)
        while not stop.is_set():
            try:
                c, _ = s.accept()
            except socket.timeout:
                continue
            try:
                with c:
                    c.settimeout(2)
                    _, n = receive_exact(c, 2)
                    receive_exact(c, n)
                    c.sendall(b"\x05\x00")
                    typ = receive_exact(c, 4)[3]
                    if typ == 3:
                        host = receive_exact(c, receive_exact(c, 1)[0]).decode()
                    elif typ == 4:
                        host = socket.inet_ntop(socket.AF_INET6, receive_exact(c, 16))
                    elif typ == 1:
                        host = socket.inet_ntoa(receive_exact(c, 4))
                    else:
                        raise ValueError(typ)
                    port = struct.unpack("!H", receive_exact(c, 2))[0]
                    seen.append([host, port])
                    c.sendall(b"\x05\x00\x00\x01" + bytes(6))
                    with socket.create_connection(
                        ("::1", 24081), timeout=2
                    ) as endpoint:
                        c.sendall(receive_exact(endpoint, 14))
            except Exception as error:
                errors.append(repr(error))


def endpoint() -> None:
    with socket.socket(socket.AF_INET6) as s:
        s.bind(("::1", 24081))
        s.listen()
        s.settimeout(0.2)
        while not stop.is_set():
            try:
                c, _ = s.accept()
            except socket.timeout:
                continue
            with c:
                c.sendall(b"IPV6_ENDPOINT!")


def dial(address: str) -> bool:
    with socket.create_connection(("127.0.0.1", 10808), timeout=2) as c:
        c.settimeout(2)
        c.sendall(b"\x05\x01\x00")
        assert receive_exact(c, 2) == b"\x05\x00"
        c.sendall(
            b"\x05\x01\x00\x04"
            + socket.inet_pton(socket.AF_INET6, address)
            + struct.pack("!H", 24081)
        )
        reply = receive_exact(c, 10)
        if reply[1] != 0:
            return False
        return receive_exact(c, 14) == b"IPV6_ENDPOINT!"


def blocked(address: str) -> None:
    count = len(seen)
    try:
        success = dial(address)
    except (OSError, EOFError, RuntimeError):
        success = False
    assert not success and len(seen) == count, (address, seen)


def disabled_family_reply() -> dict[str, int]:
    name = "disabled.test"
    identifier = os.urandom(2)
    request = (
        identifier + struct.pack("!HHHHH", 0x0100, 1, 0, 0, 0) + question_bytes(name, 1)
    )
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
        client.settimeout(2)
        client.connect(("127.0.0.1", 1054))
        client.send(request)
        packet = client.recv(65535)
    if len(packet) < 12:
        raise ValueError("Обрезан ответ отключённого семейства")
    flags = struct.unpack_from("!H", packet, 2)[0]
    rcode = flags & 15
    # Timeout/SERVFAIL/ошибка протокола не доказывают запрет семейства.
    assert rcode in (0, 3, 5), ("disabled DNS rcode", rcode)
    checked = packet[:2] + struct.pack("!H", flags & ~15) + packet[4:]
    assert not response_addresses(checked, identifier, name, 1), "IPv4 адрес получен"
    return dict(rcode=rcode, bytes=len(packet))


def guarded(worker: Callable[[], None]) -> None:
    try:
        worker()
    except Exception as error:
        errors.append(f"{worker.__name__}: {error!r}")


threads = [
    threading.Thread(target=guarded, args=(f,)) for f in (upstream, proxy, endpoint)
]
for t in threads:
    t.start()
results = []
try:
    for engine in ["xray", "sing-box"]:
        binary = validated_binary(
            Path("/home/myrs/projects/yonik/local/engines")
            / ("xray-26.10.31" if engine == "xray" else "sing-box-1.14.4")
            / engine,
            engine,
        )
        for mode in ["Real", "FakeIp"]:
            config = json.loads((base / f"{engine}-{mode}-v6.json").read_text())
            with engine_process(
                binary, engine, config, base / f"ipv6-{engine}-{mode}-{time.time_ns()}"
            ) as child:
                time.sleep(0.4)
                ttls = []
                before = len(queries)
                answers = dns_query("ipv6.test", qtype=28, port=1054, ttl_capture=ttls)
                assert len(answers) == 1, answers
                assert dial(answers[0]), seen
                assert seen[-1][0] == (
                    "ipv6.test" if mode == "FakeIp" else "2001:db8::20"
                ), seen
                if mode == "FakeIp":
                    assert len(queries) == before
                    blocked("fc00:3fff:ffff:ffff:ffff:ffff:ffff:ffff")
                before_disabled = len(queries)
                disabled = disabled_family_reply()
                assert not any(
                    row["name"] == "disabled.test" and row["qtype"] == 1
                    for row in queries[before_disabled:]
                ), "Отключённый A запрос попал в upstream"
                assert child.poll() is None
                results.append(
                    dict(
                        engine=engine,
                        mode=mode,
                        answers=answers,
                        ttl=ttls,
                        disabled=disabled,
                        socks=seen[-1],
                        unknown_blocked=mode == "FakeIp",
                    )
                )
            if mode == "FakeIp":
                with engine_process(
                    binary, engine, config, base / f"reset-{engine}-{time.time_ns()}"
                ):
                    time.sleep(0.4)
                    blocked(answers[0])
                results[-1]["empty_session_old_mapping_blocked"] = True
finally:
    stop.set()
    for t in threads:
        t.join(3)
        if t.is_alive():
            errors.append("Fixture thread не завершился")
    (base / "ipv6-results.json").write_text(
        json.dumps(dict(results=results, queries=queries, errors=errors), indent=2)
    )

assert not errors, errors
