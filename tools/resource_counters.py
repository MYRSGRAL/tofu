"""Ограниченные kernel counters. Никаких записей, perf или привилегий.

ABI: docs.kernel.org/power/powercap/powercap.html,
Documentation/ABI/testing/sysfs-class-power и kernel/sched/debug.c (proc_sched_show).
"""

from __future__ import annotations

import os
import time
from dataclasses import asdict, dataclass
from itertools import islice
from pathlib import Path
from typing import Literal

Status = Literal["available", "unavailable", "invalid_interval"]
Reason = Literal[
    "ok",
    "permission_denied",
    "unavailable",
    "malformed_or_limit",
    "baseline_required",
    "thread_churn_or_pid_reuse",
    "source_changed",
    "time",
    "reset_or_wrap",
    "wrap_bound_unknown",
    "wrap_or_delta_bound",
    "quantization",
    "not_owned_pid",
    "schedstats_disabled",
    "wakeups_field_missing",
    "schedstats_changed",
    "not_discharging",
    "power_state_changed",
    "no_sources",
]


@dataclass(frozen=True)
class Reading:
    status: Status
    reason: Reason
    value: int | None = None
    monotonic_ns: int | None = None
    scope: str | None = None
    range_uj: int | None = None
    identity: tuple[tuple[int, int], ...] = ()
    max_power_uw: int | None = None


def number(text: str) -> int:
    value = text.strip()
    if not value.isascii() or not value.isdecimal() or len(value) > 20:
        raise ValueError("counter")
    result = int(value)
    if result > (1 << 64) - 1:
        raise ValueError("overflow")
    return result


def read(path: Path) -> str:
    with path.open("rb") as handle:
        data = handle.read(65537)
    if len(data) > 65536:
        raise ValueError("read_limit")
    return data.decode("ascii").strip()


def entries(path: Path, limit: int) -> list[Path]:
    with os.scandir(path) as iterator:
        paths = [Path(entry.path) for entry in islice(iterator, limit + 1)]
    if len(paths) > limit:
        raise ValueError("entry_limit")
    return sorted(paths)


def failure(error: Exception) -> Reading:
    reason = (
        "permission_denied" if isinstance(error, PermissionError) else "unavailable"
    )
    if isinstance(error, (ValueError, UnicodeError)):
        reason = "malformed_or_limit"
    return Reading("unavailable", reason)


def interval(before: Reading, after: Reading, *, battery: bool = False) -> Reading:
    if after.status != "available":
        return after
    if before.status != "available":
        return Reading("unavailable", "baseline_required")
    reason: Reason | None = None
    if before.identity != after.identity:
        reason = "thread_churn_or_pid_reuse"
    elif before.scope != after.scope or before.range_uj != after.range_uj:
        reason = "source_changed"
    elif (
        before.monotonic_ns is None
        or after.monotonic_ns is None
        or not 0 < after.monotonic_ns - before.monotonic_ns <= 120_000_000_000
    ):
        reason = "time"
    if reason:
        return Reading("invalid_interval", reason)
    assert before.value is not None and after.value is not None
    delta = before.value - after.value if battery else after.value - before.value
    if delta < 0:
        # Один спад не отличает reset от wrap; без верхней границы мощности
        # нельзя доказать отсутствие нескольких оборотов даже при росте.
        return Reading("invalid_interval", "reset_or_wrap")
    if after.range_uj is not None and delta > 0:
        if before.max_power_uw != after.max_power_uw or not after.max_power_uw:
            return Reading("invalid_interval", "wrap_bound_unknown")
        elapsed = after.monotonic_ns - before.monotonic_ns
        upper = after.max_power_uw * elapsed / 1_000_000_000
        if upper >= after.range_uj or delta > upper:
            return Reading("invalid_interval", "wrap_or_delta_bound")
    if delta == 0 and after.scope != "process_wakeups":
        return Reading("invalid_interval", "quantization")
    return Reading("available", "ok", delta, after.monotonic_ns, after.scope)


class Collector:
    """Production roots фиксированы; fixture roots доступны только тестам.

    Хранится один baseline, максимум 512 потоков и 64 sysfs entries на уровень.
    Энергия разных зон никогда не суммируется.
    """

    def __init__(self) -> None:
        self.proc = Path("/proc")
        self.power = Path("/sys/class/powercap")
        self.battery = Path("/sys/class/power_supply")
        self.previous: dict[str, Reading] = {}

    @classmethod
    def _fixture(cls, proc: Path, power: Path, battery: Path) -> Collector:
        collector = cls()
        collector.proc, collector.power, collector.battery = proc, power, battery
        return collector

    def wakeups(self, pid: int) -> Reading:
        try:
            if type(pid) is not int or pid <= 0:
                return Reading("unavailable", "not_owned_pid")
            if number(read(self.proc / "sys/kernel/sched_schedstats")) != 1:
                return Reading("unavailable", "schedstats_disabled")
            root = self.proc / str(pid)
            if self.proc == Path("/proc"):
                stat = read(root / "stat").rsplit(")", 1)[1].split()
                if root.stat().st_uid != os.getuid() or (
                    pid != os.getpid() and int(stat[1]) != os.getpid()
                ):
                    return Reading("unavailable", "not_owned_pid")
            tasks = entries(root / "task", 512)
            if not tasks:
                raise ValueError("empty_tasks")
            identities, total = [], 0
            for task in tasks:
                first = read(task / "stat").rsplit(")", 1)[1].split()
                values = [
                    line.split(":", 1)[1]
                    for line in read(task / "sched").splitlines()
                    if line.split(":", 1)[0].strip() == "se.statistics.nr_wakeups"
                ]
                if len(values) != 1:
                    return Reading("unavailable", "wakeups_field_missing")
                total += number(values[0])
                if total > (1 << 64) - 1:
                    raise ValueError("overflow")
                second = read(task / "stat").rsplit(")", 1)[1].split()
                if first[19] != second[19]:
                    return Reading("invalid_interval", "thread_churn_or_pid_reuse")
                identities.append((number(task.name), number(first[19])))
            if entries(root / "task", 512) != tasks:
                return Reading("invalid_interval", "thread_churn_or_pid_reuse")
            if number(read(self.proc / "sys/kernel/sched_schedstats")) != 1:
                return Reading("invalid_interval", "schedstats_changed")
            return Reading(
                "available",
                "ok",
                total,
                time.monotonic_ns(),
                "process_wakeups",
                identity=tuple(identities),
            )
        except (OSError, ValueError, UnicodeError, IndexError) as error:
            return failure(error)

    def energy(self) -> dict[str, Reading]:
        result: dict[str, Reading] = {}
        try:
            zones = entries(self.power, 64)
            seen: set[Path] = set()
            # class/powercap содержит ссылки на каждую зону; вложенный обход
            # добавляет зоны control-type без неограниченного rglob.
            for parent in tuple(zones):
                if parent.is_dir() and not (parent / "energy_uj").exists():
                    zones.extend(entries(parent, 64))
            if len(zones) > 128:
                raise ValueError("zone_limit")
            for zone in zones:
                if not zone.name.startswith(("intel-rapl:", "intel-rapl-mmio:")):
                    continue
                resolved = zone.resolve()
                if resolved in seen:
                    continue
                seen.add(resolved)
                key = zone.name
                try:
                    name = read(zone / "name")
                    scope = (
                        "package"
                        if name.startswith("package-")
                        else "psys"
                        if name == "psys"
                        else "subdomain"
                    )
                    maximum = number(read(zone / "max_energy_range_uj"))
                    value = number(read(zone / "energy_uj"))
                    if maximum == 0 or value >= maximum:
                        raise ValueError("range")
                    try:
                        max_power = number(read(zone / "max_power_range_uw"))
                    except FileNotFoundError:
                        max_power = None
                    result[key] = Reading(
                        "available",
                        "ok",
                        value,
                        time.monotonic_ns(),
                        scope,
                        maximum,
                        max_power_uw=max_power,
                    )
                except (OSError, ValueError, UnicodeError) as error:
                    result[key] = failure(error)
        except (OSError, ValueError, UnicodeError) as error:
            result["powercap"] = failure(error)
        try:
            for supply in entries(self.battery, 32):
                if read(supply / "type") != "Battery":
                    continue
                key = "battery:" + supply.name
                try:
                    if read(supply / "status") != "Discharging":
                        result[key] = Reading("unavailable", "not_discharging")
                        continue
                    value = number(read(supply / "energy_now"))
                    if read(supply / "status") != "Discharging":
                        result[key] = Reading("invalid_interval", "power_state_changed")
                        continue
                    result[key] = Reading(
                        "available", "ok", value, time.monotonic_ns(), "battery_uwh"
                    )
                except (OSError, ValueError, UnicodeError) as error:
                    result[key] = failure(error)
        except (OSError, ValueError, UnicodeError) as error:
            result["battery"] = failure(error)
        return result or {"energy": Reading("unavailable", "no_sources")}

    def sample(self, pid: int) -> dict[str, object]:
        current = {"wakeups": self.wakeups(pid), **self.energy()}
        deltas = {
            key: asdict(
                interval(
                    self.previous.get(key, Reading("unavailable", "baseline_required")),
                    value,
                    battery=key.startswith("battery:"),
                )
            )
            for key, value in current.items()
        }
        self.previous = current
        return {
            "schema_version": 1,
            "energy": {
                "status": "available"
                if any(
                    v.status == "available"
                    for k, v in current.items()
                    if k != "wakeups"
                )
                else "unavailable",
                "scope": "independent_sources_never_sum",
            },
            "readings": {key: asdict(value) for key, value in current.items()},
            "intervals": deltas,
        }
