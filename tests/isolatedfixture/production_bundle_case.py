"""Actual bundle gate: настоящий helper, FD/asset ownership и foreign sentinels в VM."""

from __future__ import annotations

import fcntl
import hashlib
import json
import os
import stat
import subprocess
import time
from pathlib import Path

BASE = Path("/opt/yonik-bundle-case")


def guard() -> None:
    assert os.geteuid() == 0
    assert os.environ.get("YONIK_PRODUCTION_HELPER_VM") == "1"
    assert Path("/sys/class/dmi/id/sys_vendor").read_text().strip() == "QEMU"
    assert (
        Path("/etc/yonik-production-helper-vm").read_text()
        == "yonik-production-helper-disposable-v1\n"
    )
    assert not os.environ.get("DBUS_SYSTEM_BUS_ADDRESS")


def checked_json(args: list[str]) -> object:
    return json.loads(subprocess.check_output(args, text=True, timeout=5))


def foreign() -> dict[str, object]:
    return {
        "nft": checked_json(["nft", "-j", "list", "table", "inet", "foreign_sentinel"]),
        "link": checked_json(["ip", "-j", "link", "show", "foreign0"]),
        "address": checked_json(["ip", "-j", "addr", "show", "foreign0"]),
        "rule": [
            row
            for row in checked_json(["ip", "-j", "rule"])
            if row.get("priority") == 1234
        ],
    }


def inspect(marker: dict[str, object]) -> dict[str, object]:
    pid = int(marker["pid"])
    engine = str(marker["engine"])
    live = int(
        subprocess.check_output(
            [
                "systemctl",
                "show",
                "--property=MainPID",
                "--value",
                "yonik-worker.service",
            ],
            text=True,
            timeout=5,
        )
    )
    assert live == pid and pid > 1
    deadline = time.monotonic() + 3
    while not os.readlink(f"/proc/{pid}/exe").startswith("/memfd:yonik-engine"):
        assert time.monotonic() < deadline, "Native exec не завершён"
        time.sleep(0.02)
    worker_uid = int(
        subprocess.check_output(["id", "-u", "yonik-worker"], text=True, timeout=5)
    )
    resource = Path(f"/proc/{pid}/fd/32")
    metadata = resource.stat()
    assert stat.S_ISREG(metadata.st_mode) and metadata.st_uid == worker_uid
    assert os.readlink(resource).startswith("/memfd:yonik-r-")
    data = resource.read_bytes()
    expected = bytes(marker["digest"]).hex()
    assert len(data) == marker["bytes"] and hashlib.sha256(data).hexdigest() == expected
    with resource.open("rb") as file:
        seals = fcntl.fcntl(file.fileno(), fcntl.F_GET_SEALS)
    assert (
        seals
        & (
            fcntl.F_SEAL_SEAL
            | fcntl.F_SEAL_WRITE
            | fcntl.F_SEAL_GROW
            | fcntl.F_SEAL_SHRINK
        )
        == 15
    )
    status = Path(f"/proc/{pid}/status").read_text()
    assert "CapEff:\t0000000000001000\n" in status and "NoNewPrivs:\t1\n" in status
    descriptors = {}
    for entry in Path(f"/proc/{pid}/fd").iterdir():
        try:
            descriptors[entry.name] = os.readlink(entry)
        except FileNotFoundError:
            continue
    assert not any("memfd:yonik-engine" in target for target in descriptors.values())
    assets: dict[str, object] = {}
    if engine == "xray":
        shm = Path(f"/proc/{pid}/root/dev/shm")
        stats = os.statvfs(shm)
        assert (
            stats.f_blocks * stats.f_frsize == 64 * 1024 * 1024 and stats.f_files == 128
        )
        assert (
            subprocess.check_output(
                ["stat", "-f", "-c", "%T", str(shm)], text=True, timeout=5
            ).strip()
            == "tmpfs"
        )
        assert os.readlink(f"/proc/{pid}/ns/mnt") != os.readlink("/proc/1/ns/mnt")
        roots = [
            entry for entry in shm.iterdir() if entry.name.startswith("yonik-xray-run-")
        ]
        assert len(roots) == 1
        root = roots[0]
        ledger = json.loads((root / ".custody").read_text())
        assert (
            ledger["role"] == "run"
            and ledger["session"] > 0
            and ledger["generation"] > 0
        )
        assert ledger["cgroup"].strip() == "0::/system.slice/yonik-worker.service"
        assert (
            root.stat().st_uid == worker_uid
            and stat.S_IMODE(root.stat().st_mode) == 0o700
        )
        assert (
            root.stat().st_ino == ledger["root_ino"]
            and root.stat().st_dev == ledger["root_dev"]
        )
        entries = ledger["entries"]
        assert len(entries) == 1 and entries[0]["name"] == f"r-{expected}.dat"
        artifact = root / entries[0]["name"]
        assert (
            artifact.stat().st_ino == entries[0]["ino"]
            and artifact.stat().st_uid == worker_uid
        )
        assert stat.S_IMODE(artifact.stat().st_mode) == 0o400
        assert hashlib.sha256(artifact.read_bytes()).hexdigest() == expected
        assets = {"ledger": ledger, "mode": "0400", "worker_owned": True}
    return {
        "pid": pid,
        "engine": engine,
        "worker_uid": worker_uid,
        "sealed_owned_fd": True,
        "native_hash": expected,
        "fd_count": len(descriptors),
        "assets": assets,
        "memory": [
            line
            for line in status.splitlines()
            if line.startswith(("VmRSS:", "VmHWM:"))
        ],
    }


def main() -> int:
    guard()
    output = BASE / "results"
    output.mkdir()
    before = foreign()
    (output / "foreign-before.json").write_text(json.dumps(before, indent=2))
    environment = dict(os.environ)
    environment["YONIK_PRODUCTION_HELPER_VM"] = "1"
    command = [
        "setpriv",
        "--bounding-set=-all",
        "--inh-caps=-all",
        "--ambient-caps=-all",
        str(BASE / "installed-helper-session"),
        "production_helper_resource_bundle_both_cores",
        "--exact",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]
    inspected: set[str] = set()
    with (output / "controller.log").open("w") as log:
        with subprocess.Popen(
            command, stdout=log, stderr=subprocess.STDOUT, env=environment
        ) as child:
            deadline = time.monotonic() + 150
            while child.poll() is None and time.monotonic() < deadline:
                for engine in ["xray", "sing-box"]:
                    marker = BASE / "actual" / f"{engine}-applied.json"
                    if engine in inspected or not marker.exists():
                        continue
                    proof = inspect(json.loads(marker.read_text()))
                    (output / f"{engine}-worker.json").write_text(
                        json.dumps(proof, indent=2)
                    )
                    inspected.add(engine)
                    (BASE / "actual" / f"{engine}-inspected").touch(exist_ok=False)
                time.sleep(0.1)
            if child.poll() is None:
                child.terminate()
                child.wait(timeout=5)
                raise TimeoutError(
                    "Actual bundle controller deadline; mutations не повторяются"
                )
            code = child.returncode
    after = foreign()
    (output / "foreign-after.json").write_text(json.dumps(after, indent=2))
    result = {
        "exit": code,
        "inspected": sorted(inspected),
        "foreign_equal": before == after,
    }
    (output / "result.json").write_text(json.dumps(result, indent=2))
    assert code == 0 and inspected == {"xray", "sing-box"} and before == after
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
