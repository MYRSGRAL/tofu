"""Native RTNETLINK gate в проверенном изолированном network namespace."""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import re
import select
import signal
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from dns_fixture import verify_namespace
from engine_smoke import install_signal_handlers, run_sandbox

LOG = logging.getLogger("tofu.network-smoke")
MAX_LINE = 1024
MAX_RECORDS = 512
FIELDS = {"epoch", "routes", "resolver", "manager", "exhausted", "closed"}


class Observer:
    """Ограниченный JSON-lines reader без блокирующего readline."""

    def __init__(self, binary: Path) -> None:
        self.process = subprocess.Popen(
            [str(binary)],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env={**os.environ, "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent"},
        )
        if self.process.stdout is None:
            raise RuntimeError("Нет stdout наблюдателя")
        self.fd = self.process.stdout.fileno()
        os.set_blocking(self.fd, False)
        self.buffer = b""
        self.records: list[dict[str, Any]] = []
        self.eof = False

    def read_until(self, deadline: float) -> list[dict[str, Any]]:
        records = []
        while time.monotonic() < deadline and not self.eof:
            ready, _, _ = select.select(
                [self.fd], [], [], max(0, deadline - time.monotonic())
            )
            if not ready:
                break
            data = os.read(self.fd, MAX_LINE + 1)
            if not data:
                self.eof = True
                if self.buffer:
                    raise RuntimeError("Обрезанная запись наблюдателя")
                break
            self.buffer += data
            while b"\n" in self.buffer:
                line, self.buffer = self.buffer.split(b"\n", 1)
                if len(line) > MAX_LINE:
                    raise RuntimeError("Слишком длинная запись наблюдателя")
                record = json.loads(line)
                if (
                    not isinstance(record, dict)
                    or set(record) != FIELDS
                    or not isinstance(record["epoch"], str)
                    or re.fullmatch(r"[0-9a-f]{32}:[0-9]+", record["epoch"]) is None
                    or any(type(record[key]) is not bool for key in FIELDS - {"epoch"})
                ):
                    raise RuntimeError("Небезопасный формат метаданных")
                if len(self.records) >= MAX_RECORDS:
                    raise RuntimeError("Превышен лимит записей наблюдателя")
                self.records.append(record)
                records.append(record)
            if len(self.buffer) > MAX_LINE:
                raise RuntimeError("Слишком длинная незавершённая запись")
        return records

    def ready(self) -> str:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            self.read_until(min(deadline, time.monotonic() + 0.1))
            if self.eof:
                raise RuntimeError("Наблюдатель завершился до readiness")
            if self.records and self.records[-1]["routes"]:
                self.read_until(time.monotonic() + 0.3)
                return self.current()
        raise TimeoutError("RTNETLINK не стал готов за 5 секунд")

    def current(self) -> str:
        if not self.records or self.eof:
            raise RuntimeError("Наблюдатель не работает")
        latest = self.records[-1]
        if latest["exhausted"] or not latest["routes"] or latest["closed"]:
            raise RuntimeError("Потеряно покрытие RTNETLINK")
        return str(latest["epoch"])

    def changed(self, before: str) -> str:
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            self.read_until(min(deadline, time.monotonic() + 0.1))
            if self.current() != before:
                # Несколько сообщений одного изменения могут сливаться в watch.
                self.read_until(time.monotonic() + 0.2)
                return self.current()
        raise TimeoutError("Изменение ядра не обновило эпоху")

    def stop(self, event: signal.Signals) -> None:
        self.process.send_signal(event)
        self.read_until(time.monotonic() + 4)
        code = self.process.wait(timeout=1)
        if code != 0 or not self.eof or not self.records[-1]["closed"]:
            raise RuntimeError("Не подтверждены shutdown/join/закрытие канала")

    def cleanup(self) -> None:
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=3)
        if self.process.stdout is not None:
            self.process.stdout.close()


def ip(*arguments: str, optional: bool = False) -> bool:
    result = subprocess.run(
        ["ip", *arguments],
        check=False,
        timeout=3,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if result.returncode and not optional:
        raise RuntimeError("Не выполнена операция синтетического ip fixture")
    return result.returncode == 0


def worker(binary: Path, host_inode: int, evidence: Path) -> None:
    # Ни одна сетевая мутация не разрешена до обеих проверок verify_namespace.
    verify_namespace(host_inode)
    with binary.open("rb") as source:
        binary_sha256 = hashlib.file_digest(source, "sha256").hexdigest()
    report: dict[str, Any] = {
        "isolated": True,
        "binary_sha256": binary_sha256,
        "changes": [],
        "skipped": [],
        "lifecycle": [],
    }
    observer = Observer(binary)
    try:
        observer.ready()

        def change(name: str, *arguments: str, optional: bool = False) -> None:
            before = observer.current()
            if not ip(*arguments, optional=optional):
                report["skipped"].append(name)
                return
            try:
                after = observer.changed(before)
            except (RuntimeError, TimeoutError) as error:
                report["failure"] = {"step": name, "kind": type(error).__name__}
                evidence.mkdir(parents=True, exist_ok=True)
                (evidence / "failure-report.json").write_text(
                    json.dumps(report, ensure_ascii=False, indent=2) + "\n",
                    encoding="utf-8",
                )
                raise
            report["changes"].append({"step": name, "before": before, "after": after})

        change("link-add", "link", "add", "tofu-fixture", "type", "dummy")
        change("link-change", "link", "set", "tofu-fixture", "mtu", "1400")
        change("address4-add", "addr", "add", "192.0.2.1/32", "dev", "lo")
        change(
            "route4-add",
            "route",
            "add",
            "198.51.100.0/24",
            "dev",
            "lo",
            "table",
            "100",
        )
        change(
            "route4-change",
            "route",
            "replace",
            "198.51.100.0/24",
            "dev",
            "lo",
            "table",
            "100",
            "mtu",
            "1300",
        )
        change("rule4-add", "rule", "add", "priority", "12345", "table", "100")
        change("rule4-delete", "rule", "del", "priority", "12345", "table", "100")
        # Одинаковый priority не делает разные selectors одной policy rule.
        for suffix, destination in (("a", "203.0.113.0/25"), ("b", "203.0.113.128/25")):
            change(
                f"rule4-collision-add-{suffix}",
                "rule",
                "add",
                "priority",
                "12346",
                "to",
                destination,
                "table",
                "100",
            )
        for suffix, destination in (("a", "203.0.113.0/25"), ("b", "203.0.113.128/25")):
            change(
                f"rule4-collision-delete-{suffix}",
                "rule",
                "del",
                "priority",
                "12346",
                "to",
                destination,
                "table",
                "100",
            )
        change(
            "route4-delete",
            "route",
            "del",
            "198.51.100.0/24",
            "dev",
            "lo",
            "table",
            "100",
        )
        change("address4-delete", "addr", "del", "192.0.2.1/32", "dev", "lo")
        change("link-delete", "link", "del", "tofu-fixture")
        change("nexthop-add", "nexthop", "add", "id", "41", "blackhole", optional=True)
        if "nexthop-add" not in report["skipped"]:
            change("nexthop-delete", "nexthop", "del", "id", "41")
        change(
            "address6-add",
            "-6",
            "addr",
            "add",
            "2001:db8::1/128",
            "dev",
            "lo",
            "nodad",
            optional=True,
        )
        if "address6-add" not in report["skipped"]:
            change(
                "route6-add",
                "-6",
                "route",
                "add",
                "2001:db8:1::/64",
                "dev",
                "lo",
                "table",
                "100",
            )
            change(
                "route6-change",
                "-6",
                "route",
                "replace",
                "2001:db8:1::/64",
                "dev",
                "lo",
                "table",
                "100",
                "mtu",
                "1300",
            )
            change(
                "rule6-add", "-6", "rule", "add", "priority", "12345", "table", "100"
            )
            change(
                "rule6-delete", "-6", "rule", "del", "priority", "12345", "table", "100"
            )
            change(
                "route6-delete",
                "-6",
                "route",
                "del",
                "2001:db8:1::/64",
                "dev",
                "lo",
                "table",
                "100",
            )
            change(
                "address6-delete", "-6", "addr", "del", "2001:db8::1/128", "dev", "lo"
            )
        before = observer.current()
        for args in (("-s", "link", "show"), ("addr", "show"), ("route", "show")):
            ip(*args)
        observer.read_until(time.monotonic() + 1.2)
        if observer.current() != before:
            raise RuntimeError("Read-only ip/idle изменили эпоху")
        report["idle_stable"] = True
        observer.stop(signal.SIGTERM)
        report["lifecycle"].append(
            {"signal": "SIGTERM", "closed": True, "reaped": True}
        )
        report["records"] = len(observer.records)
    finally:
        observer.cleanup()
    observer = Observer(binary)
    try:
        observer.ready()
        observer.stop(signal.SIGINT)
        report["lifecycle"].append({"signal": "SIGINT", "closed": True, "reaped": True})
    finally:
        observer.cleanup()
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence / "report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    LOG.info(
        "Native RTNETLINK: %d изменений; idle; SIGTERM/SIGINT reap",
        len(report["changes"]),
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument(
        "--evidence",
        type=Path,
        default=Path("local/validation/network-observation/native"),
    )
    parser.add_argument("--worker-namespace", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    evidence = args.evidence.resolve()
    if args.worker_namespace is not None:
        worker(binary, args.worker_namespace, evidence)
        return 0
    run_sandbox(
        [
            "unshare",
            "--user",
            "--map-root-user",
            "--net",
            "--pid",
            "--fork",
            "--mount-proc",
            "--mount",
            sys.executable,
            str(Path(__file__).resolve()),
            str(binary),
            "--evidence",
            str(evidence),
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
