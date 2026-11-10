"""Изолированный IPv6 gate; запускается только через native_dns_modes."""

import json
import os
import socket
import struct
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path.cwd() / "tools"))
from dns_fixture import (
    Reply,
    dns_query,
    engine_process,
    make_response,
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


def upstream() -> None:
    with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as s:
        s.bind(("::1", 1053))
        s.settimeout(0.2)
        while not stop.is_set():
            try:
                packet, peer = s.recvfrom(4096)
            except socket.timeout:
                continue
            queries.append(packet.hex())
            s.sendto(
                make_response(packet, Reply(addresses=("2001:db8::20",), ttl=1)), peer
            )


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


threads = [threading.Thread(target=f) for f in (upstream, proxy, endpoint)]
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
                # Отключённое семейство не должно выдавать IPv4 адреса.
                try:
                    disabled = dns_query("disabled.test", qtype=1, port=1054)
                except (OSError, ValueError):
                    disabled = []
                assert not disabled, disabled
                assert child.poll() is None
                results.append(
                    dict(
                        engine=engine,
                        mode=mode,
                        answers=answers,
                        ttl=ttls,
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
    assert not errors, errors
finally:
    stop.set()
    for t in threads:
        t.join(3)
    (base / "ipv6-results.json").write_text(
        json.dumps(dict(results=results, queries=queries, errors=errors), indent=2)
    )
