"""Только actual guest PID1 package hooks; запуск product запрещён guards."""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path("/root/a4-package-hooks")
OUT = ROOT / "evidence"
UNITS = (
    "yonik-helper.service",
    "yonik-worker.service",
    "yonik-worker.socket",
    "yonik-worker-check.service",
    "yonik-worker-check.socket",
)
MANAGER = "org.freedesktop.systemd1.Manager"


def require_identity(
    vendor: str, executable: str, cgroup: str, actual: str, nonce: str
) -> None:
    if (vendor, executable, cgroup, actual) != (
        "QEMU",
        "/usr/lib/systemd/systemd",
        "cgroup2fs",
        nonce,
    ):
        raise ValueError("Не тот QEMU/PID1/cgroup/nonce")


def validate_reload_hook(text: str) -> None:
    required = {
        "Type = Path",
        "Operation = Install",
        "Operation = Upgrade",
        "Operation = Remove",
        "Target = usr/lib/systemd/system/*",
        "When = PostTransaction",
        "Exec = /usr/share/libalpm/scripts/systemd-hook daemon-reload-system",
    }
    lines = {
        x.strip() for x in text.splitlines() if x.strip() and not x.startswith("#")
    }
    if not required <= lines or len([x for x in lines if x.startswith("Exec =")]) != 1:
        raise ValueError("Не reviewed daemon-reload hook")


def reject_product_jobs(messages: list[dict]) -> None:
    for message in messages:
        if message.get("member") in ("JobNew", "JobRemoved") and "yonik-" in json.dumps(
            message.get("payload", {})
        ):
            raise ValueError("FAIL: product job обнаружен")


def validate_reload_messages(messages: list[dict]) -> dict:
    reject_product_jobs(messages)
    selected = [m for m in messages if m.get("interface") == MANAGER]
    calls = [
        m
        for m in selected
        if m.get("type") == "method_call" and m.get("member") == "Reload"
    ]
    signals = [
        m.get("payload", {}).get("data")
        for m in selected
        if m.get("type") == "signal" and m.get("member") == "Reloading"
    ]
    if len(calls) != 1 or signals != [[True], [False]]:
        raise ValueError("Неоднозначный/неполный Manager.Reload trace")
    return {
        "reload_calls": 1,
        "reloading": [True, False],
        "caller_unique_name": calls[0].get("sender"),
    }


def command(
    args: list[str], *, accepted: tuple[int, ...] = (0,), timeout: int = 45
) -> str:
    started = time.monotonic_ns()
    process = subprocess.Popen(
        args,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=dict(os.environ, LC_ALL="C", LANG="C"),
    )
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, stderr = process.communicate()
        with (OUT / "commands.jsonl").open("a") as stream:
            stream.write(
                json.dumps(
                    {
                        "argv": args,
                        "pid": process.pid,
                        "exit": process.returncode,
                        "timeout": True,
                        "stdout": stdout,
                        "stderr": stderr,
                    }
                )
                + "\n"
            )
        raise
    with (OUT / "commands.jsonl").open("a") as stream:
        stream.write(
            json.dumps(
                {
                    "argv": args,
                    "pid": process.pid,
                    "exit": process.returncode,
                    "start_ns": started,
                    "end_ns": time.monotonic_ns(),
                    "stdout": stdout,
                    "stderr": stderr,
                }
            )
            + "\n"
        )
    if process.returncode not in accepted:
        raise ValueError(f"Guest command exit {process.returncode}: {args}")
    return stdout.strip()


def properties(unit: str) -> dict[str, str]:
    result = command(
        [
            "systemctl",
            "show",
            unit,
            "-p",
            "LoadState",
            "-p",
            "ActiveState",
            "-p",
            "SubState",
            "-p",
            "MainPID",
            "-p",
            "CanReload",
            "-p",
            "Job",
            "-p",
            "InvocationID",
        ]
    )
    return dict(line.split("=", 1) for line in result.splitlines() if "=" in line)


def identity(nonce: str) -> dict:
    require_identity(
        Path("/sys/class/dmi/id/sys_vendor").read_text().strip(),
        str(Path("/proc/1/exe").resolve()),
        command(["stat", "-fc", "%T", "/sys/fs/cgroup"]),
        Path("/run/a4-package-hooks-ready").read_text().strip(),
        nonce,
    )
    if command(["systemd-detect-virt", "--vm"]) not in ("qemu", "kvm"):
        raise ValueError("Не QEMU/KVM")
    command(["systemd-detect-virt", "--chroot"], accepted=(1,))
    command(["systemd-notify", "--booted"])
    owner = json.loads(
        command(
            [
                "busctl",
                "--system",
                "--json=short",
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetNameOwner",
                "s",
                "org.freedesktop.systemd1",
            ]
        )
    )["data"][0]
    pid = json.loads(
        command(
            [
                "busctl",
                "--system",
                "--json=short",
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetConnectionUnixProcessID",
                "s",
                owner,
            ]
        )
    )["data"][0]
    if pid != 1:
        raise ValueError("systemd1 owner не PID1")
    return {
        "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
        "pid1_stat": Path("/proc/1/stat").read_text().split()[21],
        "manager_owner": owner,
        "manager_pid": pid,
    }


def guards(nonce: str) -> dict:
    result = identity(nonce)
    states = {}
    for unit in UNITS:
        if os.readlink("/etc/systemd/system/" + unit) != "/dev/null":
            raise ValueError("Product mask отсутствует: " + unit)
        state = properties(unit)
        if (
            any(
                state.get(k) != v
                for k, v in {
                    "LoadState": "masked",
                    "ActiveState": "inactive",
                    "SubState": "dead",
                }.items()
            )
            or state.get("MainPID", "0") != "0"
            or state.get("InvocationID")
            or state.get("Job", "").split(" ")[0] not in ("", "0")
        ):
            raise ValueError("Product unit guard: " + unit + " " + json.dumps(state))
        states[unit] = state
    owned = json.loads(
        command(
            [
                "busctl",
                "--system",
                "--json=short",
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                "s",
                "cloud.highpriority.yonik.Network1",
            ]
        )
    )["data"][0]
    if owned:
        raise ValueError("Product bus name активен")
    for path in Path("/proc").glob("[0-9]*/exe"):
        try:
            executable = str(path.resolve(strict=True))
        except (OSError, RuntimeError):
            continue
        if "/yonik/" in executable or Path(executable).name in (
            "yonik-helper",
            "yonik-worker",
            "yonik-desktop",
            "xray",
            "sing-box",
        ):
            raise ValueError("Product/core process активен")
    if command(["systemctl", "list-jobs", "--no-legend", "--no-pager"]):
        raise ValueError("Guest jobs не drained")
    dbus = properties("dbus.service")
    if (
        any(
            dbus.get(k) != v
            for k, v in {
                "LoadState": "loaded",
                "ActiveState": "active",
                "CanReload": "yes",
            }.items()
        )
        or int(dbus.get("MainPID", "0")) <= 0
    ):
        raise ValueError("Guest dbus не active/reloadable")
    result.update(units=states, dbus=dbus)
    return result


def validate_script(text: str) -> None:
    # Сравниваются именно relevant branches; посторонние operations не исполняются.
    branches = {
        "daemon-reload-system": [
            "systemd_live",
            "/usr/bin/systemctl --system daemon-reload",
        ],
        "reload": ["systemd_live", '/usr/bin/systemctl try-reload-or-restart "$@"'],
    }
    for label, expected in branches.items():
        match = re.search(
            r"^\s*" + re.escape(label) + r"\)\s*\n(.*?)\n\s*;;", text, re.M | re.S
        )
        actual = (
            []
            if not match
            else [
                line.strip()
                for line in match[1].splitlines()
                if line.strip() and not line.strip().startswith("#")
            ]
        )
        if actual != expected:
            raise ValueError("Hook script relevant branch отличается: " + label)
    functions = {
        "skip_chrooted": [
            "if systemd-detect-virt --chroot; then",
            'echo >&2 "  Skipped: Running in chroot."',
            "exit 0",
            "fi",
        ],
        "systemd_live": [
            "skip_chrooted",
            "if ! systemd-notify --booted; then",
            'echo >&2 "  Skipped: Current root is not booted."',
            "exit 0",
            "fi",
        ],
    }
    for name, expected in functions.items():
        match = re.search(r"^" + name + r"\(\) \{\n(.*?)\n\}", text, re.M | re.S)
        actual = (
            []
            if not match
            else [
                line.strip()
                for line in match[1].splitlines()
                if line.strip() and not line.strip().startswith("#")
            ]
        )
        if actual != expected:
            raise ValueError("Hook guard function differs: " + name)
    for fragment in [
        "systemd-detect-virt --chroot",
        "Skipped: Running in chroot.",
        "systemd-notify --booted",
        "Skipped: Current root is not booted.",
    ]:
        if fragment not in text:
            raise ValueError("Hook live guard отличается")


def preflight(nonce: str) -> dict:
    command(["cloud-init", "status", "--wait"], timeout=60)
    command(["pacman", "-Q", "systemd", "pacman", "dbus", "glibc"])
    command(["pacman", "-Q", "yonik"], accepted=(1,))
    result = guards(nonce)
    result["manager"] = command(
        ["systemctl", "show", "-p", "Version", "-p", "SystemState"]
    )
    hooks = {}
    for name in ["30-systemd-daemon-reload-system.hook", "dbus-reload.hook"]:
        path = Path("/usr/share/libalpm/hooks") / name
        owner = command(["pacman", "-Qo", str(path)])
        text = path.read_text()
        if name.startswith("30-"):
            validate_reload_hook(text)
        elif "Exec = /usr/share/libalpm/scripts/systemd-hook reload dbus" not in text:
            raise ValueError("DBus hook отличается")
        hooks[str(path)] = {
            "owner": owner,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "text": text,
        }
    path = Path("/usr/share/libalpm/scripts/systemd-hook")
    validate_script(path.read_text())
    for item in [path, Path("/usr/bin/systemctl")]:
        info = item.stat()
        if info.st_uid != 0 or info.st_mode & 0o022:
            raise ValueError("Unsafe real hook executable")
        hooks[str(item)] = {
            "owner": command(["pacman", "-Qo", str(item)]),
            "sha256": hashlib.sha256(item.read_bytes()).hexdigest(),
            "mode": oct(info.st_mode & 0o7777),
        }
    hooks[str(path)]["text"] = path.read_text()
    result["hooks"] = hooks
    (ROOT / "preflight-identity.json").write_text(json.dumps(result))
    return result


def phase(name: str, nonce: str) -> dict:
    binding = json.loads((ROOT / "binding.json").read_text())
    phases = [
        "install-old",
        "upgrade-candidate",
        "rollback-old",
        "return-candidate",
        "uninstall-candidate",
    ]
    donefile = ROOT / "completed.json"
    done = json.loads(donefile.read_text()) if donefile.exists() else []
    if name != phases[len(done)]:
        raise ValueError("Неверный phase order")
    baseline = json.loads((ROOT / "preflight-identity.json").read_text())
    before = guards(nonce)
    if any(before[k] != baseline[k] for k in ("boot_id", "pid1_stat", "manager_owner")):
        raise ValueError("PID1/boot сменился")
    for filename, expected in binding["packages"].items():
        if (
            hashlib.sha256((ROOT / "input" / filename).read_bytes()).hexdigest()
            != expected
        ):
            raise ValueError("Package hash изменился")
    matches = [
        "type='method_call',path='/org/freedesktop/systemd1',interface='org.freedesktop.systemd1.Manager',member='Reload'"
    ] + [
        "type='signal',path='/org/freedesktop/systemd1',interface='org.freedesktop.systemd1.Manager',member='"
        + member
        + "'"
        for member in ("Reloading", "JobNew", "JobRemoved")
    ]
    errors = []
    messages = []
    with (OUT / (name + "-monitor.stderr")).open("w+") as stderr:
        monitor = subprocess.Popen(
            [
                "busctl",
                "--system",
                "--json=short",
                *[("--match=" + m) for m in matches],
                "monitor",
            ],
            stdout=subprocess.PIPE,
            stderr=stderr,
            text=True,
            env=dict(os.environ, LC_ALL="C"),
        )

        def receive() -> None:
            assert monitor.stdout
            with (OUT / (name + "-monitor.jsonl")).open("w") as stream:
                for line in monitor.stdout:
                    try:
                        message = json.loads(line)
                        reject_product_jobs([message])
                        messages.append(message)
                        stream.write(json.dumps(message) + "\n")
                        stream.flush()
                    except (ValueError, TypeError) as error:
                        errors.append(str(error))

        reader = threading.Thread(target=receive, daemon=True)
        reader.start()
        process = None
        try:
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                stderr.flush()
                if (
                    "Monitoring bus message stream"
                    in (OUT / (name + "-monitor.stderr")).read_text()
                ):
                    break
                if monitor.poll() is not None:
                    raise ValueError("Monitor exited before readiness")
                time.sleep(0.1)
            else:
                raise ValueError("Monitor readiness отсутствует; transaction НЕ начата")
            cursor = command(["journalctl", "-n", "0", "--show-cursor", "--no-pager"])
            cursor_value = next(
                (
                    line.removeprefix("-- cursor: ")
                    for line in cursor.splitlines()
                    if line.startswith("-- cursor: ")
                ),
                None,
            )
            if not cursor_value:
                raise ValueError("Journal cursor unavailable before transaction")
            argv = [
                "pacman",
                "--config",
                str(ROOT / "pacman.conf"),
                "--root",
                "/",
                "--dbpath",
                "/var/lib/pacman",
                "--logfile",
                "/var/log/pacman.log",
                "--cachedir",
                "/var/cache/pacman/pkg",
                "--gpgdir",
                "/etc/pacman.d/gnupg",
                "--hookdir",
                "/etc/pacman.d/hooks",
                "--noconfirm",
            ]
            if name == "uninstall-candidate":
                argv += ["-R", "-dd", "yonik"]
            else:
                filename = (
                    "old.pkg.tar.zst"
                    if name in ("install-old", "rollback-old")
                    else "candidate.tar"
                )
                argv += ["-U", "-dd", str(ROOT / "input" / filename)]
            with (OUT / (name + "-pacman.log")).open("w") as log:
                process = subprocess.Popen(
                    argv, stdout=log, stderr=log, env=dict(os.environ, LC_ALL="C")
                )
                active = {
                    "phase": name,
                    "argv": argv,
                    "pid": process.pid,
                    "start_ns": time.monotonic_ns(),
                    "monitor_pid": monitor.pid,
                    "monitor_ready": True,
                    "journal_before": cursor,
                }
                (OUT / (name + "-active.json")).write_text(json.dumps(active, indent=2))
                deadline = time.monotonic() + 60
                while process.poll() is None:
                    if (
                        errors
                        or monitor.poll() is not None
                        or time.monotonic() > deadline
                    ):
                        process.terminate()
                        try:
                            process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()
                        raise ValueError(
                            "UNKNOWN transaction outcome / monitor FAIL: "
                            + json.dumps(errors)
                        )
                    time.sleep(0.05)
                active.update(exit=process.returncode, end_ns=time.monotonic_ns())
                (OUT / (name + "-active.json")).write_text(json.dumps(active, indent=2))
                if process.returncode != 0:
                    raise ValueError("Transaction FAILED; no retry")
            time.sleep(0.4)
        finally:
            monitor.terminate()
            try:
                monitor.wait(timeout=5)
            except subprocess.TimeoutExpired:
                monitor.kill()
                monitor.wait()
            reader.join(timeout=5)
        if errors:
            raise ValueError("Monitor FAIL: " + json.dumps(errors))
    trace = validate_reload_messages(messages)
    text = (OUT / (name + "-pacman.log")).read_text()
    if (
        "Skipped:" in text
        or "Reloading system manager configuration" not in text
        or "Reloading system bus configuration" not in text
    ):
        raise ValueError("Hook live branch не подтверждена")
    after = guards(nonce)
    if (
        any(after[k] != before[k] for k in ("boot_id", "pid1_stat", "manager_owner"))
        or after["dbus"]["MainPID"] != before["dbus"]["MainPID"]
    ):
        raise ValueError("Manager/DBus restarted")
    command(["journalctl", "--since", "-2min", "--no-pager", "-u", "dbus.service"])
    journal_text = command(
        ["journalctl", "--after-cursor", cursor_value, "--no-pager", "-o", "json"]
    )
    journal = [
        json.loads(line) for line in journal_text.splitlines() if line.startswith("{")
    ]
    pid1 = [row for row in journal if str(row.get("_PID")) == "1"]
    if not any("Reload" in str(row.get("MESSAGE", "")) for row in pid1):
        raise ValueError("PID1 journal reload evidence absent")
    if any(int(row.get("PRIORITY", "6")) <= 3 for row in pid1):
        raise ValueError("PID1 error within transaction; no PASS")
    (OUT / (name + "-pid1-journal.json")).write_text(json.dumps(pid1, indent=2) + "\n")
    expected_version = (
        "0.1.0.r20261110-2"
        if name == "rollback-old" or name == "install-old"
        else "0.1.0.r20261111-1"
    )
    installed = command(
        ["pacman", "-Q", "yonik"],
        accepted=(1,) if name == "uninstall-candidate" else (0,),
    )
    if name != "uninstall-candidate" and installed != "yonik " + expected_version:
        raise ValueError("Installed version mismatch")
    elfs = {}
    for path in (
        "usr/bin/yonik-desktop",
        "usr/lib/yonik/yonik-helper",
        "usr/lib/yonik/yonik-worker",
    ):
        item = Path("/") / path
        if name == "uninstall-candidate":
            if item.exists():
                raise ValueError("ELF остался после uninstall")
        else:
            info = item.stat()
            digest = hashlib.sha256(item.read_bytes()).hexdigest()
            expected = binding["elfs"][
                "old" if expected_version.endswith("-2") else "candidate"
            ][path]
            if digest != expected or info.st_uid != 0 or info.st_mode & 0o7777 != 0o755:
                raise ValueError("ELF binding/owner/mode mismatch")
            elfs[path] = {"sha256": digest, "uid": 0, "mode": "0o755"}
    result = {
        "phase": name,
        "before": before,
        "after": after,
        "trace": trace,
        "elfs": elfs,
        "installed": installed,
        "live_hook": True,
        "product_activation": False,
    }
    done.append(name)
    donefile.write_text(json.dumps(done))
    return result


def main() -> None:
    action, nonce = sys.argv[1:]
    if os.getuid() != 0 or not re.fullmatch("[0-9a-f]{32}", nonce):
        raise ValueError("Only root disposable guest with nonce")
    if Path("/run/a4-package-hooks-ready").read_text().strip() != nonce:
        raise ValueError("Nonce gate отказал до writes")
    OUT.mkdir(mode=0o700, exist_ok=True)
    try:
        if action == "preflight":
            result = preflight(nonce)
        elif action == "reconcile":
            result = {
                "installed": command(["pacman", "-Q", "yonik"], accepted=(0, 1)),
                "jobs": command(["systemctl", "list-jobs", "--no-pager"]),
                "pacman_processes": command(["pgrep", "-a", "pacman"], accepted=(0, 1)),
            }
        else:
            result = phase(action, nonce)
        (OUT / (action + "-result.json")).write_text(
            json.dumps(result, indent=2) + "\n"
        )
        print(json.dumps({"action": action, "status": "PASS"}))
    except Exception as error:
        (OUT / (action + "-failure.json")).write_text(
            json.dumps(
                {"action": action, "error": str(error), "type": type(error).__name__},
                indent=2,
            )
            + "\n"
        )
        raise


if __name__ == "__main__":
    main()
