"""Узкий units-only selector после отдельной ошибки compressed output-log."""

from __future__ import annotations

import fcntl
import json
import os
import re
import subprocess
import sys
from pathlib import Path

import a4_offline_bootstrap_extract as offline


def units_pattern() -> str:
    unit = r"(sshd|sshdgenkeys|cloud-init|cloud-init-local|cloud-init-main|cloud-init-network|cloud-config|cloud-final|systemd-time-wait-sync|systemd-timesyncd|time-sync|network|network-online|yonik-helper|yonik-worker|yonik-worker-check)\.(service|socket|target)"
    paths = [
        "",
        "usr",
        "usr/lib",
        "usr/lib/systemd",
        "usr/lib/systemd/system",
        "etc",
        "etc/systemd",
        "etc/systemd/system",
    ]
    for parent in ("usr/lib/systemd/system", "etc/systemd/system"):
        paths.extend(
            [
                parent + "/" + unit,
                parent + "/" + unit + r"\.d",
                parent + "/" + unit + r"\.d/[^/]+\.conf",
            ]
        )
    return "^/(" + "|".join(paths) + ")$"


def main() -> int:
    tag = sys.argv[1]
    if (
        not re.fullmatch("[a-f0-9]{8}", tag)
        or os.getuid() != os.geteuid()
        or os.getuid() == 0
    ):
        raise ValueError("Own extraction identity required")
    out = offline.BASE / "local/validation/a4-offline-bootstrap" / tag
    task = Path("/var/tmp/codex-myrs/transient") / ("a4-offline-extract-" + tag)
    fd = os.open(task, os.O_RDONLY | os.O_DIRECTORY)
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            pass  # Основной reader продолжает держать корень сессии.
        else:
            raise ValueError(
                "Original bounded session lock no longer held; no new session"
            )
        prepared = json.loads((out / "prepare.json").read_text())
        if (
            prepared["root"]["filesystem"] != "btrfs"
            or offline.sha(Path("/usr/bin/btrfs"))
            != prepared["inventory"]["btrfs"]["sha256"]
        ):
            raise ValueError("Filesystem/tool binding mismatch")
        image = task / "root.raw"
        if image.stat().st_size != prepared["root"]["length"]:
            raise ValueError("Partition size changed")
        destination = out / "units-only"
        destination.mkdir(mode=0o700)
        (out / "adaptive-units-source.py").write_bytes(Path(__file__).read_bytes())
        argv = [
            "/usr/bin/btrfs",
            "restore",
            "-S",
            "--path-regex",
            units_pattern(),
            str(image),
            str(destination),
        ]
        process = subprocess.Popen(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env={"PATH": "/usr/bin", "LC_ALL": "C"},
        )
        stdout, stderr = process.communicate(timeout=60)
        (out / "units-only-command.json").write_text(
            json.dumps(
                {
                    "argv": argv,
                    "pid": process.pid,
                    "exit": process.returncode,
                    "stdout": stdout,
                    "stderr": stderr,
                },
                indent=2,
            )
            + "\n"
        )
        if process.returncode:
            print(
                json.dumps(
                    {
                        "unit_read_tool_gap": True,
                        "exit": process.returncode,
                        "stderr": stderr,
                    }
                )
            )
            return 1
        nonce = json.loads(
            (
                Path(
                    json.loads(offline.SUMMARY.read_text())["failed_overlay"]["path"]
                ).parent
                / "active.json"
            ).read_text()
        )["nonce"]
        rows = []
        for item in destination.rglob("*"):
            if item.is_dir() and not item.is_symlink():
                continue
            path = "/" + str(item.relative_to(destination))
            offline.allowed_guest_path(path)
            if item.is_symlink():
                rows.append({"path": path, "symlink": os.readlink(item)})
                continue
            if (
                any(
                    parent.is_symlink()
                    for parent in item.parents
                    if parent != destination.parent
                )
                or item.stat().st_size > 4 * 1024 * 1024
            ):
                raise ValueError("Output parent/size guard")
            rows.append(
                {
                    "path": path,
                    "sha256": offline.sha(item),
                    "bytes": item.stat().st_size,
                }
            )
            text = offline.sanitize_text(item.read_text(errors="replace"), nonce)
            saved = out / "texts" / (path.lstrip("/").replace("/", "__") + ".txt")
            saved.parent.mkdir(mode=0o700, exist_ok=True)
            saved.write_text(text)
        (out / "units-only-manifest.json").write_text(json.dumps(rows, indent=2) + "\n")
        print(
            json.dumps(
                {
                    "units_only_files": len(rows),
                    "manifest": str(out / "units-only-manifest.json"),
                }
            )
        )
        return 0
    finally:
        os.close(fd)


if __name__ == "__main__":
    sys.exit(main())
