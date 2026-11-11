#!/usr/bin/env python3
"""Последовательный benchmark одного native GUI; только синтетические данные."""

from __future__ import annotations

import argparse
import json
import os
import platform
import selectors
import signal
import struct
import subprocess
import time
from pathlib import Path
from types import FrameType
from typing import Any

MAX_LINE = 8192
MAX_STDOUT = 1024 * 1024
MAX_STDERR = 1024 * 1024


class BenchmarkError(RuntimeError):
    """Измерение нельзя признать валидным."""


class Cancellation:
    """Отложенная отмена не оставляет процесс в окне spawn/cleanup."""

    def __init__(self) -> None:
        self.protected = True
        self.pending: int | None = None
        self.previous = {
            number: signal.getsignal(number)
            for number in (signal.SIGTERM, signal.SIGINT)
        }
        for number in self.previous:
            signal.signal(number, self.receive)

    def receive(self, number: int, _frame: FrameType | None) -> None:
        self.pending = number
        if not self.protected:
            self.check()

    def check(self) -> None:
        if self.pending is not None:
            raise KeyboardInterrupt(f"Получен сигнал {self.pending}")

    def restore(self) -> None:
        for number, handler in self.previous.items():
            signal.signal(number, handler)


def page_hash(rows: list[dict[str, Any]]) -> str:
    """Независимая реализация FNV-1a64 из PROTOCOL.md."""
    value = 0xCBF29CE484222325
    for row in rows:
        data = struct.pack("<I", row["id"])
        for key in ("name", "host", "country"):
            data += row[key].encode("utf-8") + b"\0"
        data += struct.pack("<IB", row["latency_ms"], row["favorite"])
        for byte in data:
            value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value:016x}"


class Oracle:
    """Состояние dataset и ожидаемая проекция report, без импорта Rust backend."""

    def __init__(self, count: int) -> None:
        countries = (
            ("DE", "Германия"),
            ("NL", "Нидерланды"),
            ("SE", "Швеция"),
            ("JP", "Япония"),
        )
        self.rows = []
        for index in range(count):
            code, name = countries[index % 4]
            self.rows.append(
                dict(
                    id=index + 1,
                    name=f"{name} · Сервер {index:05}",
                    host=f"server-{index}.example.invalid",
                    country=code,
                    latency_ms=10 + index * 17 % 180,
                    favorite=False,
                )
            )
        self.revision = 1
        self.selected: int | None = None
        self.search = ""
        self.offset = 0
        self.favorites_only = False
        self.sort = "source"
        self.scroll = 0

    def apply(self, command: dict[str, Any]) -> str:
        op = command["op"]
        if op == "query":
            self.search = command["search"]
            self.offset = command["offset"]
            self.favorites_only = command["favorites_only"]
            self.sort = command["sort"]
        elif op == "select":
            self.selected = command["id"]
        elif op == "favorite":
            self.rows[command["id"] - 1]["favorite"] = command["value"]
        elif op == "scroll":
            self.scroll = command["index"]
            return "frame"
        elif op in ("hide", "show"):
            return {"hide": "hidden", "show": "shown"}[op]
        else:
            raise BenchmarkError(f"Неизвестная команда oracle: {op}")
        self.revision += 1
        self.scroll = 0
        return "frame"

    def expected(self, event: str) -> dict[str, Any]:
        search = self.search.lower()
        rows = [
            row
            for row in self.rows
            if (not self.favorites_only or row["favorite"])
            and (search in row["name"].lower() or search in row["host"])
        ]
        if self.sort == "name":
            rows.sort(key=lambda row: (row["name"], row["id"]))
        page = rows[self.offset : self.offset + 200]
        return dict(
            event=event,
            revision=self.revision,
            total=len(rows),
            offset=self.offset,
            model_rows=len(page),
            page_hash=page_hash(page),
            first_id=page[0]["id"] if page else None,
            last_id=page[-1]["id"] if page else None,
            selected_id=self.selected,
            scroll_index=self.scroll,
        )

    def validate(self, report: dict[str, Any], event: str) -> None:
        if set(report) != set(self.expected(event)) | {
            "toolkit",
            "created_total",
            "alive_rows",
            "visible_rows",
        }:
            raise BenchmarkError("Неверный набор полей report")
        for key, expected in self.expected(event).items():
            if (
                key not in report
                or type(report[key]) is not type(expected)
                or report[key] != expected
            ):
                raise BenchmarkError(
                    f"Некорректный ack {key}: {report.get(key)!r}, ожидалось {expected!r}"
                )
        if not isinstance(report.get("toolkit"), str) or not report["toolkit"]:
            raise BenchmarkError("Отсутствует toolkit")
        for key in ("created_total", "alive_rows", "visible_rows"):
            value = report.get(key)
            if (
                key not in report
                or (value is None and key == "created_total")
                or (value is not None and (type(value) is not int or value < 0))
            ):
                raise BenchmarkError(f"Некорректный счётчик {key}")


def parse_proc(stat: str, status: str, smaps: str) -> dict[str, Any]:
    fields = stat[stat.rindex(")") + 2 :].split()
    values = {}
    for line in status.splitlines():
        key, _, value = line.partition(":")
        if key in ("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches"):
            values[key] = int(value.strip())
    memory = {}
    for line in smaps.splitlines():
        key, _, value = line.partition(":")
        if key in ("Pss", "Private_Clean", "Private_Dirty", "Private_Hugetlb"):
            memory[key] = int(value.split()[0]) * 1024
    return dict(
        cpu_ticks=int(fields[11]) + int(fields[12]),
        pss_bytes=memory["Pss"],
        uss_bytes=memory["Private_Clean"]
        + memory["Private_Dirty"]
        + memory.get("Private_Hugetlb", 0),
        main_thread_context_switches_proxy={
            key: values[key]
            for key in ("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")
        },
    )


def sample_process(pid: int) -> dict[str, Any]:
    root = Path(f"/proc/{pid}")
    try:
        result = parse_proc(
            (root / "stat").read_text(),
            (root / "status").read_text(),
            (root / "smaps_rollup").read_text(),
        )
        result["thread_names"] = sorted(
            path.read_text().strip() for path in (root / "task").glob("*/comm")
        )
    except (OSError, ValueError, KeyError, IndexError) as error:
        raise BenchmarkError(
            f"Нельзя прочитать метрики собственного PID {pid}: {error}"
        ) from error
    result.update(
        monotonic_ns=time.monotonic_ns(),
        pid=pid,
        clock_ticks_per_second=os.sysconf("SC_CLK_TCK"),
        gpu=None,
        hardware_power=None,
    )
    return result


def session_state() -> dict[str, str]:
    try:
        result = subprocess.run(
            [
                "loginctl",
                "show-session",
                os.environ.get("XDG_SESSION_ID", "self"),
                "-p",
                "Type",
                "-p",
                "Active",
                "-p",
                "LockedHint",
                "--no-pager",
            ],
            capture_output=True,
            text=True,
            timeout=3,
            check=True,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise BenchmarkError(
            f"Нельзя подтвердить состояние Wayland-сессии: {error}"
        ) from error
    return dict(
        line.split("=", 1) for line in result.stdout.splitlines() if "=" in line
    )


def require_unlocked() -> dict[str, str]:
    state = session_state()
    if (
        state.get("Type") != "wayland"
        or state.get("Active") != "yes"
        or state.get("LockedHint") != "no"
    ):
        raise BenchmarkError(f"Нужна активная разблокированная Wayland-сессия: {state}")
    return state


class Child:
    """Один owned process group; ограниченные неблокирующие каналы."""

    def __init__(self, argv: list[str], stderr_path: Path) -> None:
        self.log = stderr_path.open("xb")
        self.spawn_ns = time.monotonic_ns()
        try:
            self.process = subprocess.Popen(
                argv,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
            )
        except BaseException:
            self.log.close()
            raise
        self.stopped = False
        self.reservation_lost = False
        self.closing = False
        self.selector: selectors.BaseSelector | None = None
        self.buffer = bytearray()
        self.pending: list[dict[str, Any]] = []
        self.stdout_bytes = 0
        self.stderr_bytes = 0
        self.stderr_saved = 0
        try:
            self.selector = selectors.DefaultSelector()
            for stream, name in (
                (self.process.stdout, "stdout"),
                (self.process.stderr, "stderr"),
            ):
                assert stream is not None
                os.set_blocking(stream.fileno(), False)
                self.selector.register(stream, selectors.EVENT_READ, name)
            assert self.process.stdin is not None
            os.set_blocking(self.process.stdin.fileno(), False)
        except BaseException:
            self.close()
            raise

    def pump(self, seconds: float) -> None:
        assert self.selector is not None
        for key, _ in self.selector.select(max(0, seconds)):
            chunk = os.read(key.fd, MAX_LINE)
            if not chunk:
                self.selector.unregister(key.fileobj)
                if key.data == "stdout" and self.buffer:
                    raise BenchmarkError("Незавершённая JSONL строка")
                continue
            if key.data == "stderr":
                self.stderr_bytes += len(chunk)
                saved = chunk[: max(0, MAX_STDERR - self.stderr_saved)]
                self.log.write(saved)
                self.log.flush()
                self.stderr_saved += len(saved)
                continue
            self.stdout_bytes += len(chunk)
            if self.stdout_bytes > MAX_STDOUT:
                raise BenchmarkError("Превышен лимит stdout")
            self.buffer.extend(chunk)
            while b"\n" in self.buffer:
                line, _, rest = self.buffer.partition(b"\n")
                self.buffer = bytearray(rest)
                if len(line) > MAX_LINE:
                    raise BenchmarkError("Слишком длинная JSONL строка")
                try:
                    report = json.loads(line)
                except (ValueError, UnicodeError, RecursionError) as error:
                    raise BenchmarkError("Невалидный JSON stdout") from error
                if not isinstance(report, dict) or report.get("event") == "error":
                    raise BenchmarkError(f"Ошибка протокола: {report!r}")
                self.pending.append(report)
                if len(self.pending) > 16:
                    raise BenchmarkError("Слишком много незапрошенных ack")
            if len(self.buffer) > MAX_LINE:
                raise BenchmarkError("Слишком длинная JSONL строка")

    def exit_code(self) -> int | None:
        """Наблюдаем exit без reap: PID резервирует числовой PGID до cleanup."""
        if self.reservation_lost:
            raise BenchmarkError("Лидер уже reap: reservation PGID потеряна")
        if self.stopped:
            return self.process.returncode
        try:
            status = os.waitid(
                os.P_PID, self.process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT
            )
        except ChildProcessError as error:
            self.reservation_lost = True
            raise BenchmarkError("Внешний reap: reservation PGID потеряна") from error
        if status is None:
            return None
        return (
            status.si_status if status.si_code == os.CLD_EXITED else -status.si_status
        )

    def alive(self) -> None:
        code = self.exit_code()
        if code is not None:
            raise BenchmarkError(f"Преждевременный exit: {code}")

    def send(self, command: dict[str, Any], timeout: float) -> int:
        self.pump(0)
        if self.pending or self.buffer:
            raise BenchmarkError("Незапрошенный/частичный ack перед командой")
        self.alive()
        data = json.dumps(command, separators=(",", ":")).encode() + b"\n"
        if len(data) > 4096:
            raise BenchmarkError("Слишком длинная команда")
        started = time.monotonic_ns()
        deadline = time.monotonic() + timeout
        assert self.process.stdin is not None
        with selectors.DefaultSelector() as writer:
            writer.register(self.process.stdin, selectors.EVENT_WRITE)
            while data:
                if not writer.select(max(0, deadline - time.monotonic())):
                    raise BenchmarkError("Timeout записи stdin")
                try:
                    written = os.write(self.process.stdin.fileno(), data)
                except BrokenPipeError as error:
                    raise BenchmarkError("Закрыт stdin") from error
                data = data[written:]
        return started

    def ack(self, timeout: float) -> tuple[dict[str, Any], int]:
        deadline = time.monotonic() + timeout
        while not self.pending:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise BenchmarkError("Timeout ожидания ack")
            self.pump(min(remaining, 1.0))
            self.alive()
        if len(self.pending) != 1:
            raise BenchmarkError("Более одного ack на действие")
        return self.pending.pop(), time.monotonic_ns()

    def idle(
        self, seconds: float, visible: bool, samples: list[dict[str, Any]] | None = None
    ) -> list[dict[str, Any]]:
        deadline = time.monotonic() + seconds
        if samples is None:
            samples = []
        while True:
            self.alive()
            state = require_unlocked() if visible else session_state()
            samples.append(
                dict(process=sample_process(self.process.pid), session=state)
            )
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            interval_end = time.monotonic() + min(60, remaining)
            while time.monotonic() < interval_end:
                self.pump(interval_end - time.monotonic())
                self.alive()
                if self.pending or self.buffer:
                    raise BenchmarkError("Незапрошенный stdout во время idle")
        return samples

    def finish(self, timeout: float) -> None:
        deadline = time.monotonic() + timeout
        while self.exit_code() is None:
            if time.monotonic() >= deadline:
                raise BenchmarkError("Timeout завершения Quit")
            self.pump(min(0.1, deadline - time.monotonic()))
        assert self.selector is not None
        drain_deadline = time.monotonic() + min(1.0, timeout)
        while self.selector.get_map() and time.monotonic() < drain_deadline:
            self.pump(max(0, drain_deadline - time.monotonic()))
        code = self.exit_code()
        if self.pending or self.buffer or code != 0:
            raise BenchmarkError(f"Некорректное завершение: {code}")
        self.close()

    def close(self) -> None:
        if self.stopped or self.closing:
            return
        self.closing = True
        try:
            if self.process.returncode is not None or self.reservation_lost:
                self.stopped = True
                raise BenchmarkError("Лидер уже reap: reservation PGID потеряна")
            try:
                try:
                    self.exit_code()
                    os.killpg(self.process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                # Exit лидера не означает exit потомков. Даём всей группе
                # ограниченный TERM grace, сохраняя лидера unreaped.
                time.sleep(1)
            finally:
                try:
                    self.exit_code()
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                finally:
                    # До reap запрещаем повторные сигналы, в том числе при unwind.
                    self.stopped = True
                    if not self.reservation_lost:
                        self.process.wait(timeout=3)
        finally:
            if self.selector is not None:
                self.selector.close()
            for stream in (
                self.process.stdin,
                self.process.stdout,
                self.process.stderr,
            ):
                if stream:
                    stream.close()
            self.log.close()


def query(
    search: str = "", favorites_only: bool = False, sort: str = "source"
) -> dict[str, Any]:
    return dict(
        op="query", search=search, offset=0, favorites_only=favorites_only, sort=sort
    )


def environment() -> dict[str, Any]:
    power = {}
    for path in Path("/sys/class/power_supply").glob("*/status"):
        try:
            power[str(path)] = path.read_text().strip()
        except OSError:
            power[str(path)] = None
    return dict(
        platform=platform.platform(),
        session=require_unlocked(),
        environment={
            key: os.environ.get(key)
            for key in (
                "XDG_SESSION_TYPE",
                "XDG_CURRENT_DESKTOP",
                "WAYLAND_DISPLAY",
                "QT_QPA_PLATFORM",
                "GDK_BACKEND",
                "SLINT_BACKEND",
                "SLINT_RENDERER",
            )
        },
        power_supply=power,
        power_profile=read_optional(Path("/sys/firmware/acpi/platform_profile")),
    )


def read_optional(path: Path) -> str | None:
    try:
        return path.read_text().strip()
    except OSError:
        return None


def run(args: argparse.Namespace) -> dict[str, Any]:
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    evidence_path = output / "evidence.json"
    if evidence_path.exists() or (output / "stderr.log").exists():
        raise BenchmarkError("Output уже содержит результаты; выберите новый каталог")
    evidence: dict[str, Any] = dict(
        valid=False,
        rows=args.rows,
        binary=str(Path(args.binary).resolve()),
        idle_seconds=args.idle_seconds,
        actions=[],
        idle={},
        limitations=[
            "frame/shown ack не заменяет screenshot QA и проверку toolkit callback",
            "main_thread_context_switches_proxy — переключения только главного потока, НЕ wakeups и не сумма всех потоков",
            "GPU и hardware power недоступны",
            "lock проверяется на границах интервалов <=60 секунд; краткая блокировка между проверками может быть не замечена",
        ],
    )
    child = None
    cancellation = Cancellation()
    try:
        cancellation.check()
        cancellation.protected = False
        evidence["conditions"] = environment()
        oracle = Oracle(args.rows)
        cancellation.protected = True
        child = Child(
            [str(Path(args.binary).resolve()), "--rows", str(args.rows)],
            output / "stderr.log",
        )
        cancellation.protected = False
        cancellation.check()
        report, received = child.ack(args.timeout)
        oracle.validate(report, "frame")
        toolkit = report["toolkit"]
        evidence["first_frame"] = dict(
            spawn_monotonic_ns=child.spawn_ns,
            received_monotonic_ns=received,
            latency_ns=received - child.spawn_ns,
            report=report,
            process=sample_process(child.process.pid),
        )
        commands = [
            query(),
            query("server-0.example.invalid"),
            dict(op="select", id=1),
            dict(op="favorite", id=1, value=True),
            query(favorites_only=True),
            query(),
            query(sort="name"),
            dict(op="scroll", index=199),
        ]

        def action(command: dict[str, Any]) -> None:
            expected_event = oracle.apply(command)
            started = child.send(command, args.timeout)
            ack, completed = child.ack(args.timeout)
            oracle.validate(ack, expected_event)
            if ack["toolkit"] != toolkit:
                raise BenchmarkError("Toolkit изменился внутри запуска")
            evidence["actions"].append(
                dict(
                    command=command,
                    sent_monotonic_ns=started,
                    received_monotonic_ns=completed,
                    latency_ns=completed - started,
                    report=ack,
                    process=sample_process(child.process.pid),
                )
            )

        for command in commands:
            action(command)
        evidence["idle"]["visible"] = []
        child.idle(args.idle_seconds, True, evidence["idle"]["visible"])
        action(dict(op="hide"))
        evidence["idle"]["hidden"] = []
        child.idle(args.idle_seconds, False, evidence["idle"]["hidden"])
        action(dict(op="show"))
        child.send(dict(op="quit"), args.timeout)
        child.finish(args.timeout)
        evidence["valid"] = True
    except (BenchmarkError, OSError, KeyboardInterrupt) as error:
        evidence["error"] = str(error) or type(error).__name__
    finally:
        cancellation.protected = True
        try:
            if child:
                try:
                    child.close()
                except (OSError, subprocess.SubprocessError) as error:
                    evidence["valid"] = False
                    evidence["cleanup_error"] = str(error)
                evidence["stderr"] = dict(
                    bytes_seen=child.stderr_bytes,
                    bytes_saved=child.stderr_saved,
                    truncated=child.stderr_bytes > child.stderr_saved,
                )
                evidence["exit_code"] = child.process.returncode
            if cancellation.pending is not None:
                evidence["valid"] = False
                evidence["error"] = f"Получен сигнал {cancellation.pending}"
            with evidence_path.open("x") as handle:
                json.dump(evidence, handle, ensure_ascii=False, indent=2)
                handle.write("\n")
        finally:
            cancellation.restore()
    return evidence


def positive(value: str) -> float:
    result = float(value)
    if not 0 < result < float("inf"):
        raise argparse.ArgumentTypeError("Нужно конечное число >0")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--rows", type=int, choices=(1000, 10000), required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--idle-seconds", type=positive, default=900)
    parser.add_argument("--timeout", type=positive, default=30)
    args = parser.parse_args()
    try:
        evidence = run(args)
    except (BenchmarkError, OSError) as error:
        parser.exit(1, f"{error}\n")
    return 0 if evidence["valid"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
