"""Изолированная native проверка TTL TCP/UDP resolution pin; полный G-DNS закрыт."""

from __future__ import annotations

import argparse
import ipaddress
import json
import logging
import socket
import struct
import subprocess
import sys
import threading
import time
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from dns_complete_smoke import SOURCE_COMMIT, digest
from dns_dual_stack_gap import IPV4, IPV6, connect_marker, marker_servers, native_config
from dns_fixture import Cname, DnsFixture, Reply, Soa, engine_process, validated_binary
from engine_compile import socks_reply, wait_listener
from engine_smoke import install_signal_handlers, run_sandbox
from network_topology_smoke import bounded_run, guard_namespace, namespace
from traffic_echo import receive_exact

LOG = logging.getLogger("yonik.resolution-pin")
PATCH_ROOT = Path(__file__).resolve().parents[1] / "adapters/sing-box/v1.14.4"
PATCHES = (
    ("strict-complete-dns.patch", "strict_complete_dns"),
    ("resolution-provenance.patch", "dns_provenance"),
    ("resolution-pin.patch", "pin_resolution"),
)
BLACKHOLE = "198.51.100.11"
_WORKER_BOOTSTRAP = """
import json, logging, runpy, sys
from pathlib import Path
sys.path.insert(0, str(Path(sys.argv[1]).parent))
m = runpy.run_path(sys.argv[1], run_name="yonik_resolution_pin_worker")
logging.basicConfig(level=logging.INFO, format="%(message)s")
m["install_signal_handlers"]()
m["worker"](Path(sys.argv[2]), Path(sys.argv[3]), tuple(json.loads(sys.argv[4])), Path(sys.argv[5]))
"""


@dataclass(frozen=True)
class Case:
    name: str
    ttl: int = 60
    cname_ttl: int | None = None
    soa_minimum: int | None = None
    strategy: str = "ipv4_only"
    delayed: bool = False
    opcode: int = 0
    reserved: bool = False

    @property
    def minimum_ttl(self) -> int:
        return min(
            value
            for value in (self.ttl, self.cname_ttl, self.soa_minimum)
            if value is not None
        )


CASES = (
    Case("both_valid", strategy="prefer_ipv4"),
    Case("ipv6_only", strategy="ipv6_only"),
    Case("ipv4_only"),
    Case("aaaa_nodata", strategy="prefer_ipv4", soa_minimum=60),
    Case("ttl_zero", ttl=0),
    Case("non_query_opcode", opcode=2),
    Case("reserved_z", reserved=True),
    Case("cname_zero", cname_ttl=0),
    Case("ttl_between_attempts", ttl=1, delayed=True),
    Case("cname_between_attempts", cname_ttl=1, delayed=True),
    Case("soa_between_attempts", soa_minimum=1, strategy="prefer_ipv4", delayed=True),
)


def expected_marker(case: Case, pin: bool | None) -> bool:
    return pin is not True or (
        case.minimum_ttl > 0
        and not case.delayed
        and case.opcode == 0
        and not case.reserved
    )


def validate_artifact(binary: Path, manifest_path: Path) -> Path:
    if manifest_path.stat().st_size > 65536:
        raise ValueError("Слишком большой adapter manifest")
    value = json.loads(manifest_path.read_bytes())
    expected = [
        {"name": name, "sha256": digest(PATCH_ROOT / name), "feature": feature}
        for name, feature in PATCHES
    ]
    if (
        not isinstance(value, dict)
        or value.get("engine") != "sing-box"
        or value.get("source_commit") != SOURCE_COMMIT
        or value.get("binary_sha256") != digest(binary)
        or value.get("patch_sha256") != expected[0]["sha256"]
        or value.get("patches") != expected
        or value.get("strict_dns_gate") is not False
        or not isinstance(value.get("adapter_features"), dict)
        or any(
            value["adapter_features"].get(feature) is not True for _, feature in PATCHES
        )
    ):
        raise ValueError("Не подтверждены бинарник и полная цепочка resolution pin")
    return binary.resolve(strict=True)


def configuration(
    case: Case, pin: bool | None, udp_connect: bool | None = None
) -> dict[str, Any]:
    value = native_config()
    resolve = value["route"]["rules"][0]
    resolve["strategy"] = case.strategy
    if pin is not None:
        resolve["pin_resolution"] = pin
    route: dict[str, Any] = {"action": "route", "outbound": "marker"}
    if udp_connect is not None:
        route["udp_connect"] = udp_connect
    value["route"]["rules"] = [resolve, route]
    value["outbounds"][0]["connect_timeout"] = "1500ms"
    return value


def answer(case: Case, name: str, qtype: int, _index: int) -> Reply:
    if name != "partial.test" or qtype not in (1, 28):
        raise ValueError("Неожиданный синтетический DNS question")
    if qtype == 28 and case.soa_minimum is not None:
        return Reply(soa=Soa("test", 60, case.soa_minimum))
    addresses = (
        ((BLACKHOLE, IPV4) if case.delayed else (IPV4,)) if qtype == 1 else (IPV6,)
    )
    return Reply(
        addresses,
        ttl=case.ttl,
        opcode=case.opcode,
        reserved=case.reserved,
        cname=None if case.cname_ttl is None else Cname("target.test", case.cname_ttl),
    )


def setup_namespace(host: tuple[int, int]) -> tuple[int, int]:
    owned = {"lo"}
    isolated = guard_namespace(host, owned)

    def mutate(*args: str) -> None:
        guard_namespace(host, owned, expected=isolated)
        code, _ = bounded_run(["ip", *args], timeout=3)
        if code:
            raise RuntimeError("Ошибка создания изолированного namespace fixture")

    mutate("link", "set", "lo", "up")
    mutate("addr", "add", f"{IPV4}/32", "dev", "lo")
    mutate("-6", "addr", "add", f"{IPV6}/128", "dev", "lo", "nodad")
    mutate("link", "add", "pin-a", "type", "veth", "peer", "name", "pin-b")
    owned.update({"pin-a", "pin-b"})
    mutate("addr", "add", "192.0.2.1/24", "dev", "pin-a")
    mutate("link", "set", "pin-a", "up")
    mutate("link", "set", "pin-b", "up")
    mutate("route", "add", f"{BLACKHOLE}/32", "via", "192.0.2.2", "dev", "pin-a")
    return isolated


def reset_delay_neighbor(host: tuple[int, int], isolated: tuple[int, int]) -> None:
    # ARP от предыдущей попытки иначе может завершиться до нового TTL.
    guard_namespace(host, {"lo", "pin-a", "pin-b"}, expected=isolated)
    code, _ = bounded_run(["ip", "neigh", "flush", "dev", "pin-a"], timeout=3)
    if code:
        raise RuntimeError("Не сброшено состояние ARP собственного fixture")


def decode_udp(packet: bytes) -> tuple[str, int, bytes]:
    if len(packet) > 4096 or len(packet) < 4 or packet[:3] != b"\0\0\0":
        raise ValueError("Некорректный SOCKS UDP header/fragment")
    if packet[3] == 3:
        if len(packet) < 5 or not 0 < packet[4] <= 253 or len(packet) < packet[4] + 7:
            raise ValueError("Обрезанный SOCKS UDP domain endpoint")
        end = 5 + packet[4]
        name = packet[5:end].decode("ascii").lower()
        if name not in {"partial.test", "partial.test."}:
            raise ValueError("Неожиданное SOCKS UDP имя fixture")
        return (
            name.rstrip("."),
            struct.unpack_from("!H", packet, end)[0],
            packet[end + 2 :],
        )
    if packet[3] not in (1, 4):
        raise ValueError("Некорректный SOCKS UDP тип endpoint")
    size, family = (4, socket.AF_INET) if packet[3] == 1 else (16, socket.AF_INET6)
    if len(packet) < size + 6:
        raise ValueError("Обрезанный SOCKS UDP endpoint")
    return (
        socket.inet_ntop(family, packet[4 : 4 + size]),
        struct.unpack_from("!H", packet, 4 + size)[0],
        packet[6 + size :],
    )


@contextmanager
def udp_markers() -> Iterator[list[dict[str, Any]]]:
    stop = threading.Event()
    hits: list[dict[str, Any]] = []
    sockets: list[socket.socket] = []
    threads: list[threading.Thread] = []

    def serve(listener: socket.socket, address: str) -> None:
        while not stop.is_set():
            try:
                payload, peer = listener.recvfrom(4096)
            except socket.timeout:
                continue
            if len(hits) >= 16:
                break
            hits.append(
                {"address": address, "payload": payload.hex(), "time": time.monotonic()}
            )
            listener.sendto(payload, peer)

    try:
        for family, address in ((socket.AF_INET, IPV4), (socket.AF_INET6, IPV6)):
            listener = socket.socket(family, socket.SOCK_DGRAM)
            sockets.append(listener)
            listener.bind((address, 24081))
            listener.settimeout(0.1)
            thread = threading.Thread(target=serve, args=(listener, address))
            threads.append(thread)
            thread.start()
        yield hits
    finally:
        stop.set()
        for thread in threads:
            thread.join(timeout=2)
        for listener in sockets:
            listener.close()
        if any(thread.is_alive() for thread in threads):
            raise RuntimeError("UDP marker thread не завершился")


def udp_probe() -> list[dict[str, Any]]:
    observations = []
    with socket.create_connection(("127.0.0.1", 10808), timeout=2) as control:
        control.settimeout(2)
        control.sendall(b"\x05\x01\x00")
        if receive_exact(control, 2) != b"\x05\x00":
            raise RuntimeError("SOCKS greeting отклонён")
        control.sendall(b"\x05\x03\x00\x01\x00\x00\x00\x00\x00\x00")
        relay = socks_reply(control)
        if (
            ipaddress.ip_address(relay[0]) != ipaddress.ip_address("127.0.0.1")
            or not 0 < relay[1] <= 65535
        ):
            raise RuntimeError("SOCKS relay вне fixture loopback")
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as datagram:
            datagram.settimeout(0.7)
            datagram.connect(relay)
            for index in range(2):
                if index:
                    time.sleep(1.2)
                payload = f"SYNTHETIC_PIN_UDP_{index}".encode()
                datagram.send(
                    b"\0\0\0\x03\x0cpartial.test" + struct.pack("!H", 24081) + payload
                )
                observation: dict[str, Any] = {
                    "sent": payload.hex(),
                    "time": time.monotonic(),
                    "received": False,
                }
                try:
                    address, port, returned = decode_udp(datagram.recv(4097))
                    if (
                        address not in {IPV4, IPV6, "partial.test"}
                        or port != 24081
                        or returned != payload
                    ):
                        raise RuntimeError("UDP marker endpoint/payload mismatch")
                    observation.update(received=True, reply_endpoint=address)
                except (TimeoutError, ConnectionError):
                    pass
                observations.append(observation)
    return observations


def worker(binary: Path, baseline: Path, host: tuple[int, int], output: Path) -> None:
    isolated = setup_namespace(host)
    results = []
    variants = (
        ("upstream", baseline, None),
        ("patched_default", binary, None),
        ("patched_false", binary, False),
        ("pin", binary, True),
    )
    for variant, engine, pin in variants:
        for case in CASES:
            guard_namespace(host, {"lo", "pin-a", "pin-b"}, expected=isolated)
            directory = output / variant / case.name
            if case.delayed:
                reset_delay_neighbor(host, isolated)
            with (
                DnsFixture(lambda *args: answer(case, *args)) as fixture,
                marker_servers() as hits,
            ):
                with engine_process(
                    engine, "sing-box", configuration(case, pin), directory
                ) as process:
                    wait_listener(process, 10808)
                    started = time.monotonic()
                    observed, error = connect_marker("partial.test")
                    elapsed = time.monotonic() - started
                    alive = process.poll() is None
                reaped = process.returncode is not None
                expected = expected_marker(case, pin)
                families = (
                    {1}
                    if case.strategy == "ipv4_only"
                    else {28}
                    if case.strategy == "ipv6_only"
                    else {1, 28}
                )
                passed = (
                    observed == expected
                    and bool(hits) == expected
                    and len(hits) == int(expected)
                    and alive
                    and reaped
                    and {r["qtype"] for r in fixture.records} == families
                )
                if case.delayed and expected:
                    passed &= elapsed >= 1.0
                result = dict(
                    variant=variant,
                    case=case.name,
                    protocol="tcp",
                    passed=passed,
                    marker=observed,
                    hits=list(hits),
                    elapsed=elapsed,
                    socks_error=error,
                    alive=alive,
                    reaped=reaped,
                    dns_records=list(fixture.records),
                )
            results.append(result)
            (directory / "observation.json").write_text(
                json.dumps(result, indent=2) + "\n"
            )
            if not passed:
                raise RuntimeError(
                    f"TCP resolution pin mismatch: {variant}/{case.name}"
                )
        for strategy in ("ipv4_only", "ipv6_only"):
            for udp_connect in (False, True):
                case = Case("udp_expiry", ttl=1, strategy=strategy)
                directory = output / variant / f"udp_{strategy}_{udp_connect}"
                guard_namespace(host, {"lo", "pin-a", "pin-b"}, expected=isolated)
                with (
                    DnsFixture(lambda *args: answer(case, *args)) as fixture,
                    udp_markers() as hits,
                ):
                    with engine_process(
                        engine,
                        "sing-box",
                        configuration(case, pin, udp_connect),
                        directory,
                    ) as process:
                        wait_listener(process, 10808)
                        observations = udp_probe()
                        alive = process.poll() is None
                    reaped = process.returncode is not None
                    expected_count = 1 if pin is True else 2
                    family = 1 if strategy == "ipv4_only" else 28
                    passed = (
                        [o["received"] for o in observations] == [True, pin is not True]
                        and [h["payload"] for h in hits]
                        == [o["sent"] for o in observations[:expected_count]]
                        and all(
                            h["address"] == (IPV4 if family == 1 else IPV6)
                            for h in hits
                        )
                        and alive
                        and reaped
                        and {r["qtype"] for r in fixture.records} == {family}
                    )
                    result = dict(
                        variant=variant,
                        case=case.name,
                        protocol="udp",
                        strategy=strategy,
                        udp_connect=udp_connect,
                        passed=passed,
                        observations=observations,
                        hits=list(hits),
                        alive=alive,
                        reaped=reaped,
                        dns_records=list(fixture.records),
                    )
                results.append(result)
                (directory / "observation.json").write_text(
                    json.dumps(result, indent=2) + "\n"
                )
                if not passed:
                    raise RuntimeError(
                        f"UDP resolution pin mismatch: {variant}/{strategy}/{udp_connect}"
                    )
    # Отдельные потоки используют cache, но не продлевают исходный deadline.
    for explicit in (True, False):
        cache_case = Case(
            "proof_cache_deadline", ttl=1, soa_minimum=60, strategy="prefer_ipv4"
        )
        directory = output / (
            "proof_cache_explicit" if explicit else "proof_cache_rules"
        )
        config = configuration(cache_case, True)
        config["dns"]["disable_cache"] = False
        if not explicit:
            del config["route"]["rules"][0]["server"]
        guard_namespace(host, {"lo", "pin-a", "pin-b"}, expected=isolated)
        with (
            DnsFixture(lambda *args: answer(cache_case, *args)) as fixture,
            marker_servers() as hits,
        ):
            with engine_process(binary, "sing-box", config, directory) as process:
                wait_listener(process, 10808)
                time.sleep(2)  # Завершить начальные события native network monitor.
                observations = []
                for index in range(3):
                    if index:
                        time.sleep(0.7)
                    observations.append(connect_marker("partial.test")[0])
                alive = process.poll() is None
            reaped = process.returncode is not None
            counts = {
                kind: sum(r["qtype"] == kind for r in fixture.records)
                for kind in (1, 28)
            }
            passed = (
                observations == [True, True, True]
                and len(hits) == 3
                and counts == {1: 2, 28: 1}
                and alive
                and reaped
            )
            result = dict(
                variant="pin",
                case="proof_cache_deadline",
                explicit_transport=explicit,
                protocol="tcp",
                passed=passed,
                markers=observations,
                hits=list(hits),
                dns_counts={"A": counts[1], "AAAA": counts[28]},
                alive=alive,
                reaped=reaped,
                dns_records=list(fixture.records),
            )
        results.append(result)
        (directory / "observation.json").write_text(json.dumps(result, indent=2) + "\n")
        if not passed:
            raise RuntimeError(f"Native proof cache deadline mismatch: {explicit}")
    report = dict(
        source_commit=SOURCE_COMMIT,
        binary_sha256=digest(binary),
        baseline_sha256=digest(baseline),
        pin_resolution_passed=True,
        strict_dns_gate=False,
        isolated=True,
        namespace=isolated,
        cases=results,
    )
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    LOG.info("Native resolution pin: %d сценариев; полный G-DNS закрыт", len(results))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = validate_artifact(args.binary, args.manifest)
    baseline = validated_binary(args.baseline, "sing-box")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    run_sandbox(
        [
            "unshare",
            "--user",
            "--map-root-user",
            "--keep-caps",
            "--net",
            "--pid",
            "--fork",
            "--kill-child",
            "--mount",
            "--mount-proc",
            sys.executable,
            "-c",
            _WORKER_BOOTSTRAP,
            str(Path(__file__).resolve()),
            str(binary),
            str(baseline),
            json.dumps(namespace()),
            str(output),
        ],
        timeout=240,
    )
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        LOG.error("Native resolution pin не пройден: %s", error)
        raise SystemExit(1) from None
