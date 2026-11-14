"""Same-UID prefix: копии actual installed dependencies и private session bus."""

from __future__ import annotations

import hashlib
import json
import re
import shutil
import subprocess
import time
from pathlib import Path


def populate(root: Path, out: Path) -> None:
    clean_env = {"PATH": "/usr/bin", "LANG": "C.UTF-8"}
    sources: set[Path] = set()
    initial = [
        Path("/usr/bin") / name
        for name in (
            "bash",
            "dbus-daemon",
            "busctl",
            "ksecretd",
            "kwalletd6",
            "systemd-detect-virt",
            "systemd-notify",
            "systemd-sysusers",
            "systemd-tmpfiles",
            "systemctl",
            "update-desktop-database",
            "sleep",
            "python",
        )
    ]
    initial += list(Path("/usr/lib/qt6/plugins/platforms").glob("libq*offscreen*.so"))
    initial += list(Path("/usr/lib/qt6/plugins/platforms").glob("libqvnc.so"))
    initial += [p for p in Path("/usr/lib/qt6/qml").rglob("*") if p.is_file()]
    initial += [Path("/usr/share/libalpm/scripts/systemd-hook")]
    initial += [
        Path("/usr/share/libalpm/hooks") / name
        for name in (
            "20-systemd-sysusers.hook",
            "30-systemd-daemon-reload-system.hook",
            "dbus-reload.hook",
            "update-desktop-database.hook",
        )
    ]
    initial += [
        p for p in Path("/usr/share/fonts/TTF").glob("DejaVuSans*") if p.is_file()
    ]
    candidate = Path(
        "/home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-mtree4/yonik-desktop"
    )
    linked = subprocess.check_output(["ldd", str(candidate)], text=True, env=clean_env)
    initial += [
        Path(p) for p in re.findall(r"(/[^\s()]+)", linked) if Path(p).is_file()
    ]
    initial += [
        p
        for p in Path("/usr/lib/python3.14").rglob("*")
        if p.is_file()
        and "__pycache__" not in p.parts
        and "site-packages" not in p.parts
    ]
    initial += [p for p in Path("/etc/fonts").rglob("*") if p.is_file()]
    pending = list(initial)
    # ldd запускается только для доверенных установленных ELF, не caller input.
    while pending:
        path = pending.pop()
        if path.parts[1:2] in (("lib",), ("lib64",)):
            path = Path("/usr/lib") / path.relative_to("/" + path.parts[1])
        if path in sources or not path.is_file():
            continue
        sources.add(path)
        with path.open("rb") as stream:
            elf = stream.read(4) == b"\x7fELF"
        if elf:
            result = subprocess.run(
                ["ldd", str(path)], capture_output=True, text=True, env=clean_env
            )
            if result.returncode == 0:
                pending.extend(
                    Path(p)
                    for p in re.findall(r"(/[^\s()]+)", result.stdout)
                    if Path(p).is_file()
                )
    rows = []
    assert all(
        path.is_relative_to("/usr") or path.is_relative_to("/etc/fonts")
        for path in sources
    )
    for path in sorted(sources):
        destination = root / str(path).lstrip("/")
        destination.parent.mkdir(parents=True, exist_ok=True)
        # Дереференсируются только actual installed source links, path остаётся own.
        shutil.copy2(path, destination)
        checksum = hashlib.sha256(path.read_bytes()).hexdigest()
        assert hashlib.sha256(destination.read_bytes()).hexdigest() == checksum
        rows.append(
            {
                "path": str(path),
                "resolved": str(path.resolve()),
                "sha256": checksum,
                "mode": oct(path.stat().st_mode & 0o7777),
            }
        )
    ownership = subprocess.run(
        ["pacman", "-Qoq", *[str(p.resolve()) for p in sorted(sources)]],
        capture_output=True,
        text=True,
        env=clean_env,
    )
    (out / "dependency-package-owners.log").write_text(
        ownership.stdout + ownership.stderr
    )
    assert ownership.returncode == 0
    (out / "dependency-files.json").write_text(json.dumps(rows, indent=2) + "\n")
    for name in ("bin", "lib", "lib64"):
        target = root / name
        target.symlink_to("usr/bin" if name == "bin" else "usr/lib")
    (root / "usr/bin/sh").symlink_to("bash")
    for name in (
        "run",
        "tmp",
        "proc",
        "dev",
        "etc",
        "home/fixture/runtime",
        "home/fixture/data",
        "home/fixture/config",
        "home/fixture/storage",
    ):
        (root / name).mkdir(parents=True, exist_ok=True)
    (root / "home/fixture/runtime").chmod(0o700)
    (root / "etc/passwd").write_text(
        "root:x:0:0:root:/root:/usr/bin/bash\nfixture:x:1000:1000:fixture:/home/fixture:/usr/bin/bash\n"
    )
    (root / "etc/group").write_text("root:x:0:\nfixture:x:1000:\n")
    (root / "etc/os-release").write_bytes(Path("/etc/os-release").read_bytes())
    # Отсутствуют host NSS/PAM/keyring/activation paths и host runtime sockets.


def attempt(root: Path, out: Path) -> dict:
    config = root / "home/fixture/bus.conf"
    config.write_text(
        '<busconfig><type>session</type><listen>unix:path=/home/fixture/runtime/bus</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
    )
    script = root / "home/fixture/run.sh"
    script.write_text("""#!/usr/bin/bash
set -u
/usr/bin/dbus-daemon --nofork --config-file=/home/fixture/bus.conf > /home/fixture/bus.log 2>&1 &
bus=$!
/usr/bin/sleep 0.5
/usr/bin/ksecretd > /home/fixture/provider.log 2>&1 &
provider=$!
/usr/bin/yonik-desktop --data-dir /home/fixture/storage > /home/fixture/gui.log 2>&1 &
gui=$!
trap 'kill "$gui" "$provider" "$bus" 2>/dev/null; wait' EXIT
# Время для actual startup; внешний supervisor сохраняет состояние и завершает группу.
wait "$gui"
""")
    args = [
        "bwrap",
        "--unshare-all",
        "--die-with-parent",
        "--bind",
        str(root),
        "/",
        "--ro-bind",
        str(root / "usr"),
        "/usr",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin",
        "--setenv",
        "HOME",
        "/home/fixture",
        "--setenv",
        "XDG_RUNTIME_DIR",
        "/home/fixture/runtime",
        "--setenv",
        "XDG_CONFIG_HOME",
        "/home/fixture/config",
        "--setenv",
        "XDG_DATA_HOME",
        "/home/fixture/data",
        "--setenv",
        "DBUS_SESSION_BUS_ADDRESS",
        "unix:path=/home/fixture/runtime/bus",
        "--setenv",
        "QT_QPA_PLATFORM",
        "offscreen",
        "--setenv",
        "QT_QUICK_BACKEND",
        "software",
        "--setenv",
        "QT_QUICK_CONTROLS_STYLE",
        "Basic",
        "/usr/bin/bash",
        "/home/fixture/run.sh",
    ]
    with (out / "runtime-supervisor.log").open("w") as log:
        process = subprocess.Popen(args, stdout=log, stderr=log, start_new_session=True)
        time.sleep(4)
        result = {"argv": args, "pid": process.pid, "exit_before_stop": process.poll()}
        address = "unix:path=" + str(root / "home/fixture/runtime/bus")
        probe = subprocess.run(
            [
                "busctl",
                "--address=" + address,
                "--json=short",
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetNameOwner",
                "s",
                "org.freedesktop.secrets",
            ],
            capture_output=True,
            text=True,
        )
        result["secret_service_probe"] = {
            "exit": probe.returncode,
            "stdout": probe.stdout,
            "stderr": probe.stderr,
        }
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        result["supervisor_exit"] = process.returncode
    for path in (root / "home/fixture").glob("*.log"):
        shutil.copyfile(path, out / path.name)
    (out / "runtime-attempt.json").write_text(json.dumps(result, indent=2) + "\n")
    return result
