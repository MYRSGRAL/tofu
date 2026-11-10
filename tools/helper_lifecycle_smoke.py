"""Проверка systemd lifecycle на безвредных процессах текущего пользователя."""

from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path
from typing import Any


def identity(pid: int) -> str | None:
    """starttime защищает проверку от повторного использования PID."""
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rpartition(") ")[2].split()
    except FileNotFoundError:
        return None
    return fields[19]


def run(
    arguments: list[str], *, check: bool = True
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        arguments, check=check, text=True, capture_output=True, timeout=10
    )


def records(directory: Path) -> list[dict[str, Any]]:
    path = directory / "events.jsonl"
    if not path.exists():
        return []
    # Последняя строка может ещё дописываться новым worker.
    contents = path.read_text()
    lines = contents.splitlines()
    if contents and not contents.endswith("\n"):
        lines.pop()
    if len(lines) > 8:
        raise RuntimeError("Неожиданное число запусков fixture")
    return [json.loads(line) for line in lines]


def worker(directory: Path) -> None:
    previous = records(directory)
    old_reaped = all(
        identity(record[role]["pid"]) != record[role]["starttime"]
        for record in previous
        for role in ("leader", "child")
    )
    if not old_reaped:
        (directory / "overlap").write_text("Старый процесс дожил до нового старта\n")
        raise RuntimeError("Предыдущая группа ещё существует")
    read_fd, write_fd = os.pipe()
    child_pid = os.fork()
    if child_pid == 0:
        os.close(read_fd)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        os.write(write_fd, b"ready")
        os.close(write_fd)
        while True:
            signal.pause()
    os.close(write_fd)
    try:
        if os.read(read_fd, 5) != b"ready":
            raise RuntimeError("Дочерний процесс не подтвердил готовность")
    finally:
        os.close(read_fd)
    event = {
        "generation": len(previous) + 1,
        "leader": {"pid": os.getpid(), "starttime": identity(os.getpid())},
        "child": {"pid": child_pid, "starttime": identity(child_pid)},
        "old_reaped_before_effect_marker": old_reaped,
    }
    with (directory / "events.jsonl").open("a", encoding="utf-8") as output:
        output.write(json.dumps(event) + "\n")
        output.flush()
        os.fsync(output.fileno())
    while True:
        signal.pause()


def await_generation(directory: Path, count: int) -> list[dict[str, Any]]:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if (directory / "overlap").exists():
            raise RuntimeError("Новый запуск обнаружил старые процессы")
        result = records(directory)
        if len(result) >= count:
            return result
        time.sleep(0.05)
    raise TimeoutError(f"Нет поколения {count} за 10 секунд")


def smoke() -> dict[str, Any]:
    availability = run(["systemctl", "--user", "is-system-running"], check=False)
    if availability.stdout.strip() not in {"running", "degraded"}:
        return {"result": "BLOCKED", "reason": "User systemd недоступен"}
    unit = f"yonik-helper-lifecycle-{uuid.uuid4().hex}.service"
    result: dict[str, Any] = {"unit": unit, "network_effects": False}
    with tempfile.TemporaryDirectory(prefix="yonik-helper-lifecycle-") as temporary:
        directory = Path(temporary)
        launched = False
        try:
            # Имя unit и команда заданы самим тестом; shell и сетевые команды не используются.
            launched = True
            run(
                [
                    "systemd-run",
                    "--user",
                    f"--unit={unit}",
                    "--property=Type=exec",
                    "--property=Restart=on-failure",
                    "--property=RestartSec=1s",
                    "--property=KillMode=control-group",
                    "--property=SendSIGKILL=yes",
                    "--property=TimeoutStopSec=1s",
                    "--property=RuntimeMaxSec=30s",
                    "--property=StartLimitIntervalSec=120s",
                    "--property=StartLimitBurst=3",
                    "/usr/bin/python3",
                    str(Path(__file__).resolve()),
                    "--worker",
                    str(directory),
                ]
            )
            first = await_generation(directory, 1)[0]
            if identity(first["child"]["pid"]) != first["child"]["starttime"]:
                raise RuntimeError("Исходный дочерний процесс уже исчез")
            run(
                [
                    "systemctl",
                    "--user",
                    "kill",
                    "--kill-whom=main",
                    "--signal=SIGKILL",
                    unit,
                ]
            )
            events = await_generation(directory, 2)
            if len(events) != 2:
                raise RuntimeError("Неожиданное число перезапусков")
            result.update(result="PASS", events=events)
        finally:
            if launched:
                # Останавливается только созданный этим вызовом unit, включая его children.
                stopped = run(["systemctl", "--user", "stop", unit], check=False)
                state = run(
                    ["systemctl", "--user", "show", "--property=ActiveState", unit],
                    check=False,
                )
                gone = all(
                    identity(event[role]["pid"]) != event[role]["starttime"]
                    for event in records(directory)
                    for role in ("leader", "child")
                )
                run(["systemctl", "--user", "reset-failed", unit], check=False)
                if (
                    stopped.returncode
                    or state.stdout.strip()
                    not in {"ActiveState=inactive", "ActiveState=failed"}
                    or not gone
                ):
                    raise RuntimeError(f"Не подтверждена очистка {unit}")
                result["cleanup"] = (
                    "PASS: unit остановлен, все записанные identities исчезли"
                )
    return result


def main() -> int:
    if len(sys.argv) == 3 and sys.argv[1] == "--worker":
        worker(Path(sys.argv[2]))
        return 0
    if len(sys.argv) != 1:
        raise SystemExit("Запуск: python3 tools/helper_lifecycle_smoke.py")
    try:
        result = smoke()
    except (OSError, RuntimeError, TimeoutError, subprocess.SubprocessError) as error:
        print(json.dumps({"result": "FAIL", "error": str(error)}, ensure_ascii=False))
        return 1
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if result["result"] == "PASS" else 2


if __name__ == "__main__":
    raise SystemExit(main())
