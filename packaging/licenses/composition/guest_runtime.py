"""Только disposable guest: private Xvfb/DBus/netns, maps и opened QML files."""

from __future__ import annotations

import hashlib
import json
import os
import re
import signal
import subprocess
import time
from pathlib import Path


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main() -> None:
    output = Path("/home/builder/runtime")
    output.mkdir(exist_ok=True)
    binary = Path("/opt/preserved/usr/bin/yonik-desktop")
    environment = {
        **os.environ,
        "HOME": "/home/builder/runtime-home",
        "XDG_CONFIG_HOME": "/home/builder/runtime-home/config",
        "XDG_DATA_HOME": "/home/builder/runtime-home/data",
        "XDG_CACHE_HOME": "/home/builder/runtime-home/cache",
        "XDG_RUNTIME_DIR": "/home/builder/runtime-home/run",
        "QT_QPA_PLATFORM": "xcb",
        "QT_QUICK_BACKEND": "software",
        "LIBGL_ALWAYS_SOFTWARE": "1",
        "QT_DEBUG_PLUGINS": "1",
        "QML_IMPORT_TRACE": "1",
        "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent",
    }
    Path(environment["XDG_RUNTIME_DIR"]).mkdir(parents=True, mode=0o700, exist_ok=True)
    snapshots = []
    paths = set()
    with (output / "qt.log").open("w") as log:
        process = subprocess.Popen(
            [
                "strace",
                "-ff",
                "-e",
                "trace=openat,openat2",
                "-o",
                str(output / "opens"),
                str(binary),
            ],
            env=environment,
            stdout=log,
            stderr=log,
            start_new_session=True,
        )
        try:
            for index in range(4):
                time.sleep(3)
                for entry in Path("/proc").iterdir():
                    if not entry.name.isdigit():
                        continue
                    try:
                        if (entry / "exe").resolve() != binary:
                            continue
                        maps = (entry / "maps").read_text()
                        (output / f"maps-{index}-{entry.name}.txt").write_text(maps)
                        snapshots.append({"sample": index, "pid": int(entry.name)})
                        for line in maps.splitlines():
                            fields = line.split(maxsplit=5)
                            if len(fields) == 6 and fields[5].startswith("/"):
                                paths.add(fields[5])
                    except (OSError, ValueError):
                        continue
                if process.poll() is not None:
                    break
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
    mapped = set(paths)
    for trace in output.glob("opens.*"):
        for line in trace.read_text().splitlines():
            match = re.search(r'"(/[^"\\]+)".*= ([0-9]+)', line)
            if match:
                path = match[1]
                if "/qt6/" in path or path.endswith((".qml", "qmldir", ".rcc")):
                    paths.add(path)
    records = []
    for name in sorted(paths):
        path = Path(name)
        if not path.is_file():
            records.append({"path": name, "unresolved": "не regular file"})
            continue
        owner = (
            subprocess.run(
                ["pacman", "-Qoq", str(path)], capture_output=True, text=True
            )
            .stdout.strip()
            .splitlines()
        )
        version = (
            subprocess.run(
                ["pacman", "-Q", *owner], capture_output=True, text=True
            ).stdout.strip()
            if owner
            else ""
        )
        records.append(
            {
                "path": name,
                "resolved_path": str(path.resolve()),
                "sha256": sha(path),
                "observed": "maps" if name in mapped else "open",
                "owner": owner,
                "package_version": version,
            }
        )
    report = {
        "binary_sha256": sha(binary),
        "snapshots": snapshots,
        "files": records,
        "process_exit": process.returncode,
        "limits": "Один startup в Xvfb/software; прочие платформы/пути UI не покрыты. "
        "Новые guest packages не объявляются исторической runtime средой package78.",
    }
    (output / "runtime.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    )
    if not snapshots:
        raise RuntimeError("не получены maps работающего desktop")


if __name__ == "__main__":
    main()
