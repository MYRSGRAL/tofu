"""Native topology/route-get gate только в собственном network namespace."""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import logging
import os
import selectors
import signal
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from engine_smoke import install_signal_handlers, run_sandbox

MAX_OUTPUT = 16 * 1024 * 1024
LOG = logging.getLogger("tofu.topology-smoke")

# Worker вызывается только launcher внутри unshare. Публичного CLI-пути с
# переданным пользователем «host inode» нет: такая цифра не доказывает изоляцию.
_WORKER_BOOTSTRAP = """
import json, logging, runpy, sys
from pathlib import Path
sys.path.insert(0, str(Path(sys.argv[1]).parent))
module = runpy.run_path(sys.argv[1], run_name="tofu_topology_worker")
logging.basicConfig(level=logging.INFO, format="%(message)s")
module["install_signal_handlers"]()
module["worker"](Path(sys.argv[2]), tuple(json.loads(sys.argv[4])), Path(sys.argv[3]))
"""


def decode_output(data: bytes) -> dict[str, Any]:
    if len(data) > MAX_OUTPUT:
        raise ValueError("Превышен лимит вывода")

    def reject_constant(_: str) -> None:
        raise ValueError("Неконечное JSON число")

    value = json.loads(data, parse_constant=reject_constant)
    if not isinstance(value, dict):
        raise ValueError("Ожидался один JSON объект")
    return value


def bounded_run(
    command: list[str], data: bytes | None = None, *, timeout: float = 7
) -> tuple[int, bytes]:
    """Ограничиваем память во время чтения, а не после communicate."""
    process = subprocess.Popen(
        command,
        stdin=subprocess.PIPE if data is not None else subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    output = bytearray()
    deadline = time.monotonic() + timeout
    try:
        assert process.stdout is not None
        os.set_blocking(process.stdout.fileno(), False)
        pending = memoryview(data or b"")
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            if process.stdin is not None:
                os.set_blocking(process.stdin.fileno(), False)
                if pending:
                    selector.register(process.stdin, selectors.EVENT_WRITE)
                else:
                    process.stdin.close()
            reading = True
            while reading:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("Истёк срок дочерней команды")
                events = selector.select(remaining)
                if not events:
                    raise TimeoutError("Истёк срок дочерней команды")
                for key, _ in events:
                    if key.fileobj is process.stdin:
                        try:
                            count = os.write(key.fd, pending[:4096])
                            pending = pending[count:]
                        except BrokenPipeError:
                            pending = pending[:0]
                        if not pending:
                            selector.unregister(process.stdin)
                            process.stdin.close()
                    else:
                        chunk = os.read(
                            key.fd, min(65536, MAX_OUTPUT + 1 - len(output))
                        )
                        if not chunk:
                            reading = False
                            break
                        output.extend(chunk)
                        if len(output) > MAX_OUTPUT:
                            raise ValueError("Превышен лимит вывода")
        return process.wait(timeout=max(0.01, deadline - time.monotonic())), bytes(
            output
        )
    finally:
        if process.poll() is None:
            process.kill()
        process.wait(timeout=2)
        if process.stdout is not None:
            process.stdout.close()
        if process.stdin is not None and not process.stdin.closed:
            process.stdin.close()


def ip_json(*args: str) -> list[dict[str, Any]]:
    code, output = bounded_run(["ip", "-j", *args])
    if code or len(output) > MAX_OUTPUT:
        raise RuntimeError("Не выполнено независимое чтение ip")
    value = json.loads(output)
    if not isinstance(value, list):
        raise ValueError("Неверный JSON ip")
    return value


def namespace() -> tuple[int, int]:
    value = os.stat("/proc/self/ns/net")
    return value.st_dev, value.st_ino


def guard_namespace(
    host: tuple[int, int],
    owned: set[str],
    *,
    expected: tuple[int, int] | None = None,
) -> tuple[int, int]:
    actual = namespace()
    if actual[1] == host[1] or (expected is not None and actual != expected):
        raise RuntimeError("Нет подтверждённой изоляции network namespace")
    if {link["ifname"] for link in ip_json("link", "show")} != owned:
        raise RuntimeError("Namespace содержит чужие интерфейсы")
    return actual


def table_id(value: Any) -> int:
    known = {"main": 254, "local": 255, "default": 253}
    return known[str(value)] if str(value) in known else int(value)


def worker(binary: Path, host: tuple[int, int], evidence: Path) -> None:
    owned = {"lo"}
    isolated = guard_namespace(host, owned)
    report: dict[str, Any] = {"isolated": True, "checks": [], "availability": {}}
    with binary.open("rb") as source:
        report["binary_sha256"] = hashlib.file_digest(source, "sha256").hexdigest()

    def mutate(*args: str, optional: bool = False) -> bool:
        report["stage"] = "mutation:" + ":".join(args[:2])
        guard_namespace(host, owned, expected=isolated)
        code, _ = bounded_run(["ip", *args])
        if code and not optional:
            raise RuntimeError("Не выполнена мутация синтетического fixture")
        return code == 0

    def inspect(request: dict[str, Any]) -> dict[str, Any]:
        before = ip_json("link", "show")
        code, output = bounded_run([str(binary)], json.dumps(request).encode())
        value = decode_output(output)
        if code or ip_json("link", "show") != before:
            raise RuntimeError("Инспекция изменила интерфейсы или аварийно завершилась")
        if value.get("ok") and value.get("namespace") != {
            "device": isolated[0],
            "inode": isolated[1],
        }:
            raise RuntimeError("Неверная идентичность namespace")
        return value

    def snapshot() -> dict[str, Any]:
        report["stage"] = "snapshot"
        value = inspect({"op": "snapshot"})
        if not value.get("ok"):
            raise RuntimeError("Native topology вернула ошибку")
        links = ip_json("link", "show")
        actual = {(v["index"], v["name"], v["mtu"]) for v in value["links"]}
        expected = {(v["ifindex"], v["ifname"], v["mtu"]) for v in links}
        if actual != expected:
            raise RuntimeError("Расхождение links/MTU с ip-J")
        addresses = {
            (link["ifindex"], info["local"], info["prefixlen"])
            for link in ip_json("address", "show")
            for info in link.get("addr_info", [])
        }
        typed = {
            (a["index"], a["local"] or a["address"], a["prefix_len"])
            for a in value["addresses"]
        }
        if typed != addresses:
            raise RuntimeError("Расхождение адресов с ip-J")
        for family, number in (("-4", 2), ("-6", 10)):
            routes = ip_json(family, "route", "show", "table", "all")
            rules = ip_json(family, "rule", "show")
            typed_routes = [r for r in value["routes"] if r["family"] == number]
            typed_rules = [r for r in value["rules"] if r["family"] == number]
            if len(typed_routes) != len(routes) or len(typed_rules) != len(rules):
                raise RuntimeError("Расхождение количества routes/rules")
            oracle_keys = {
                (
                    table_id(r.get("table", "main")),
                    str(
                        ipaddress.ip_network(
                            r["dst"]
                            if r["dst"] != "default"
                            else ("0.0.0.0/0" if number == 2 else "::/0")
                        )
                    ),
                )
                for r in routes
            }
            if {(r["table"], r["destination"]) for r in typed_routes} != oracle_keys:
                raise RuntimeError("Расхождение таблиц/префиксов")
            if {(r["priority"] or 0, r["table"]) for r in typed_rules} != {
                (r.get("priority"), table_id(r.get("table", 0))) for r in rules
            }:
                raise RuntimeError("Расхождение приоритетов/таблиц rules")
        unknown = sum(
            len(v["unknown"])
            for key in ("links", "addresses", "routes", "rules", "nexthops")
            for v in value[key]
        )
        unknown += sum(
            len(n["unknown"]) for r in value["routes"] for n in r["multipath"]
        )
        if value["unsupported_count"] != unknown or value["incomplete"] != bool(
            unknown
        ):
            raise RuntimeError("Потеряны неизвестные атрибуты")
        report["unsupported_count"] = unknown
        return value

    def lookup(name: str, destination: str, **context: Any) -> None:
        report["stage"] = name
        query = {
            "op": "lookup",
            "destination": destination,
            "uid": os.getuid(),
            **context,
        }
        args = ["-6" if ":" in destination else "-4", "route", "get", destination]
        for key, option in (
            ("source", "from"),
            ("uid", "uid"),
            ("mark", "mark"),
            ("tos", "tos"),
            ("transport", "ipproto"),
            ("source_port", "sport"),
            ("destination_port", "dport"),
        ):
            if key in query:
                args += [option, str(query[key])]
        code, output = bounded_run(["ip", "-j", *args])
        actual = inspect(query)
        if code:
            if (
                actual.get("ok")
                or not isinstance(actual.get("errno"), int)
                or actual["errno"] <= 0
            ):
                raise RuntimeError("Отказ ядра потерян или заменён предположением")
        else:
            oracle = json.loads(output)[0]
            if not actual.get("ok"):
                raise RuntimeError("Успешный route-get oracle не совпал с native")
            index = {v["ifname"]: v["ifindex"] for v in ip_json("link", "show")}
            if actual["output_interface"] != index.get(oracle.get("dev")) or actual[
                "table"
            ] != table_id(oracle.get("table", "main")):
                raise RuntimeError("Route-get выбрал другую таблицу/интерфейс")
            if actual["gateway"] != oracle.get("gateway") or actual[
                "preferred_source"
            ] != oracle.get("prefsrc"):
                raise RuntimeError("Route-get gateway/prefsrc не совпали")
            if actual["kind"].lower() != oracle.get("type", "unicast"):
                raise RuntimeError("Route-get kind не совпал")
            for key in ("uid", "mark", "tos", "source_port", "destination_port"):
                if actual["query"][key] != query.get(
                    key, 0 if key in ("mark", "tos") else None
                ):
                    raise RuntimeError("Потерян контекст route-get")
        report["checks"].append(name)

    try:
        mutate("link", "set", "lo", "up")
        for name in ("tofu-a", "tofu-b"):
            mutate("link", "add", name, "type", "dummy")
            owned.add(name)
            mutate("link", "set", name, "up")
        mutate("link", "add", "tofu-v0", "type", "veth", "peer", "name", "tofu-v1")
        owned.update(("tofu-v0", "tofu-v1"))
        for name in ("tofu-v0", "tofu-v1"):
            mutate("link", "set", name, "up")
        for name, address in (("tofu-a", "192.0.2.1/24"), ("tofu-b", "192.0.3.1/24")):
            mutate("address", "add", address, "dev", name)
        for table, dev in (("100", "tofu-a"), ("200", "tofu-b")):
            mutate("route", "add", "198.51.100.0/24", "dev", dev, "table", table)
        mutate(
            "rule",
            "add",
            "priority",
            "80",
            "ipproto",
            "udp",
            "dport",
            "5353",
            "table",
            "100",
        )
        mutate(
            "rule",
            "add",
            "priority",
            "81",
            "ipproto",
            "tcp",
            "dport",
            "443",
            "table",
            "100",
        )
        mutate("rule", "add", "priority", "90", "from", "192.0.2.1/32", "table", "100")
        mutate("rule", "add", "priority", "100", "fwmark", "42", "table", "100")
        uid = str(os.getuid())
        mutate(
            "rule", "add", "priority", "110", "uidrange", f"{uid}-{uid}", "table", "200"
        )
        mutate("route", "add", "unreachable", "203.0.113.128/25")
        mutate(
            "route",
            "add",
            "198.18.0.0/24",
            "nexthop",
            "via",
            "192.0.2.2",
            "dev",
            "tofu-a",
            "weight",
            "1",
            "nexthop",
            "via",
            "192.0.3.2",
            "dev",
            "tofu-b",
            "weight",
            "2",
        )
        nexthops = mutate(
            "nexthop",
            "add",
            "id",
            "10",
            "via",
            "192.0.2.2",
            "dev",
            "tofu-a",
            optional=True,
        )
        report["availability"]["nexthop_objects"] = nexthops
        if nexthops:
            mutate("nexthop", "add", "id", "11", "via", "192.0.3.2", "dev", "tofu-b")
            mutate("nexthop", "add", "id", "12", "group", "10/11")
            mutate("route", "add", "198.19.0.0/24", "nhid", "12")
        ipv6 = (
            Path("/proc/sys/net/ipv6/conf/all/disable_ipv6").exists()
            and Path("/proc/sys/net/ipv6/conf/all/disable_ipv6").read_text().strip()
            == "0"
        )
        report["availability"]["ipv6"] = ipv6
        if ipv6:
            mutate("-6", "address", "add", "2001:db8:1::1/64", "dev", "tofu-a", "nodad")
            mutate("-6", "route", "add", "2001:db8:2::/64", "dev", "tofu-a")
        value = snapshot()
        selected = {r["priority"]: r for r in value["rules"] if r["family"] == 2}
        if (
            selected[100]["fwmark"] != 42
            or selected[110]["uid_range"] != [os.getuid(), os.getuid()]
            or selected[90]["source"] != "192.0.2.1/32"
            or selected[80]["destination_ports"] != [5353, 5353]
            or selected[80]["ip_protocol"] != 17
            or selected[81]["destination_ports"] != [443, 443]
            or selected[81]["ip_protocol"] != 6
        ):
            raise RuntimeError("Потеряны селекторы policy rules")
        ecmp = [r for r in value["routes"] if r["destination"] == "198.18.0.0/24"]
        if len(ecmp) != 1 or sorted(n["weight"] for n in ecmp[0]["multipath"]) != [
            1,
            2,
        ]:
            raise RuntimeError("Потерян ECMP")
        if nexthops:
            oracle = ip_json("nexthop", "show")
            if {n["id"] for n in value["nexthops"]} != {n["id"] for n in oracle}:
                raise RuntimeError("Потеряны nexthop objects")
            group = next(n for n in value["nexthops"] if n["id"] == 12)
            if {n["id"] for n in group["group"]} != {10, 11} or not any(
                r["nexthop_id"] == 12 for r in value["routes"]
            ):
                raise RuntimeError("Потеряны group references")
        report["checks"].append("snapshot-policy-ecmp")
        lookup("uid", "198.51.100.8")
        lookup("fwmark", "198.51.100.8", mark=42)
        lookup("source", "198.51.100.8", source="192.0.2.1")
        lookup(
            "tcp-ports",
            "198.51.100.8",
            transport="tcp",
            source_port=12345,
            destination_port=443,
        )
        lookup(
            "udp-ports",
            "198.51.100.8",
            transport="udp",
            source_port=12345,
            destination_port=5353,
        )
        lookup("ecmp", "198.18.0.8")
        lookup("blocked", "203.0.113.200")
        lookup("no-route", "203.0.113.8")
        if nexthops:
            lookup("nexthop-group", "198.19.0.8")
        if ipv6:
            lookup("ipv6", "2001:db8:2::8", source="2001:db8:1::1")
        mutate("link", "set", "tofu-a", "mtu", "1400")
        mutate("route", "del", "198.51.100.0/24", "table", "100")
        mutate("link", "del", "tofu-v0")
        owned.difference_update(("tofu-v0", "tofu-v1"))
        value = snapshot()
        if any(
            r["table"] == 100 and r["destination"] == "198.51.100.0/24"
            for r in value["routes"]
        ):
            raise RuntimeError("Снимок сохранил удалённый маршрут")
        report["checks"].append("fresh-update-delete")
        for request in (
            {"op": "snapshot", "timeout_ms": 0},
            {"op": "snapshot", "timeout_ms": 5001},
            {"op": "lookup", "destination": "198.51.100.8"},
            {
                "op": "lookup",
                "destination": "198.51.100.8",
                "uid": os.getuid(),
                "source_port": 80,
            },
        ):
            if inspect(request).get("ok"):
                raise RuntimeError("Невалидный запрос принят")
        for data in (b"{", b"x" * (16 * 1024 + 1)):
            code, output = bounded_run([str(binary)], data)
            if code or decode_output(output).get("ok"):
                raise RuntimeError("Некорректный stdin принят")
        report["checks"].append("invalid-input-timeout")
        report["lifecycle"] = []
        for event in (signal.SIGINT, signal.SIGTERM):
            report["stage"] = event.name
            process = subprocess.Popen(
                [str(binary)],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
            )
            try:
                try:
                    process.wait(timeout=0.2)
                    raise RuntimeError("Инспектор не дождался stdin")
                except subprocess.TimeoutExpired:
                    pass
                process.send_signal(event)
                process.wait(timeout=2)
                assert process.stdout is not None
                output = process.stdout.read(MAX_OUTPUT + 1)
                if (
                    process.returncode != 0
                    or decode_output(output).get("error") != "Cancelled"
                ):
                    raise RuntimeError("Сигнал не отменил ожидание stdin")
                report["lifecycle"].append({"signal": event.name, "reaped": True})
            finally:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=2)
                if process.stdin is not None:
                    process.stdin.close()
                if process.stdout is not None:
                    process.stdout.close()
        report["stage"] = "complete"
        report["passed"] = True
    except (
        OSError,
        ValueError,
        RuntimeError,
        TimeoutError,
        subprocess.SubprocessError,
    ) as error:
        report["passed"] = False
        report["failure_type"] = type(error).__name__
        raise
    finally:
        evidence.mkdir(parents=True, exist_ok=True)
        (evidence / "report.json").write_text(
            json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
    LOG.info(
        "Native topology: %d проверок; изоляция подтверждена", len(report["checks"])
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument(
        "--evidence",
        type=Path,
        default=Path("local/validation/network-topology/native"),
    )
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    evidence = args.evidence.resolve()
    host = namespace()
    run_sandbox(
        [
            "unshare",
            "--user",
            "--map-root-user",
            "--net",
            "--pid",
            "--fork",
            "--mount",
            "--mount-proc",
            sys.executable,
            "-c",
            _WORKER_BOOTSTRAP,
            str(Path(__file__).resolve()),
            str(binary),
            str(evidence),
            json.dumps(host),
        ],
        timeout=40,
    )
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    try:
        raise SystemExit(main())
    except (
        OSError,
        ValueError,
        RuntimeError,
        TimeoutError,
        subprocess.SubprocessError,
    ) as error:
        LOG.error(
            "Проверка не пройдена: %s",
            type(error).__name__
            if isinstance(error, subprocess.SubprocessError)
            else error,
        )
        raise SystemExit(1) from None
