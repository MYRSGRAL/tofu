"""Own-root package phases с закрытыми readers и existing guarded witness."""

from __future__ import annotations

import json
import os
import shutil
import sys
from pathlib import Path

import a4_candidate_lifecycle as common
from a4_userdata_control import rpc
from a4_userdata_prefix import sha
from a4_userdata_session import NEW

BASE = Path(__file__).resolve().parents[1]
OUT = BASE / "docs/evidence/a4-nonempty-lifecycle-142/run1"
PHASES = ("upgrade", "rollback", "candidate-reinstall", "uninstall")


def snapshot(home: Path) -> tuple[dict, dict[str, bytes]]:
    """Публичные hashes только storage/config; data/password сравниваются opaque."""
    public = {}
    opaque = {}
    for name in ("storage", "config", "data"):
        directory = home / name
        for path in sorted([directory, *directory.rglob("*")]):
            metadata = path.lstat()
            assert not path.is_symlink()
            relative = str(path.relative_to(home))
            row = {
                "mode": oct(metadata.st_mode & 0o7777),
                "uid": metadata.st_uid,
                "gid": metadata.st_gid,
                "type": "directory" if path.is_dir() else "file",
            }
            if path.is_file():
                row["size"] = metadata.st_size
                if name == "data":
                    opaque[relative] = path.read_bytes()
                else:
                    row["sha256"] = sha(path)
            public[relative] = row
    opaque["keyring-password"] = (home / "keyring-password").read_bytes()
    return public, opaque


def no_storage_readers(pid: int) -> list[dict]:
    """Проверяет только собственное дерево процессов; никаких ptrace/сигналов."""
    pending = [pid]
    rows = []
    while pending:
        current = pending.pop()
        process = Path("/proc") / str(current)
        assert process.stat().st_uid == os.getuid()
        comm = (process / "comm").read_text().strip()
        assert comm not in {
            "yonik-desktop",
            "yonik-helper",
            "yonik-worker",
        } and not comm.startswith("a4_prefix_wit"), (
            "Product/witness должен завершиться до package phase"
        )
        pending.extend(
            map(int, (process / "task" / str(current) / "children").read_text().split())
        )
        for descriptor in (process / "fd").iterdir():
            try:
                target = os.readlink(descriptor)
            except FileNotFoundError:
                continue
            if target.startswith("/home/fixture/storage/"):
                rows.append({"pid": current, "fd": descriptor.name, "path": target})
    assert not rows, rows
    return rows


def witness(root: Path, stage: str) -> dict:
    result = rpc(root, {"action": "witness", "stage": stage})
    assert result["exit"] == 0
    value = json.loads(result["stdout"])
    assert value == {
        "source": "14229843f9a11bf98ec4f5992df00f3247102ea2",
        "count": 2,
        "original_matches": True,
        "revision_matches": True,
        "metadata_matches": True,
        "same_unlocked_item": True,
        "provider_uid_pid_verified": True,
    }
    return result


def phase(state: dict, root: Path, name: str) -> dict:
    assert rpc(root, {"action": "status"})["gui_exit"] in (None, 0)
    phase_file = OUT / "package-phases.json"
    completed = json.loads(phase_file.read_text()) if phase_file.exists() else []
    assert name == PHASES[len(completed)]
    before_witness = witness(root, name + "-before")
    public_before = rpc(root, {"action": "inspect"})
    readers_before = no_storage_readers(state["pid"])
    before, secret_before = snapshot(root / "home/fixture")
    package = common.OLD if name == "rollback" else NEW
    expected = (
        "e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0"
        if name == "rollback"
        else "26a013747e4c1dbb95e7290cd37b981f5dc7ef891a8865897f09fc45bacd0237"
    )
    assert sha(package) == expected
    common.OUT = OUT
    common.command(
        [*state["pacman"], "-R", "-dd", "yonik"]
        if name == "uninstall"
        else [*state["pacman"], "-U", "-dd", str(package)]
    )
    after, secret_after = snapshot(root / "home/fixture")
    # Byte comparison ДО нового witness/inspect: API open не маскирует package effect.
    equal = before == after and secret_before == secret_after
    interim = {
        "phase": name,
        "before": before,
        "after": after,
        "userstate_equal": equal,
    }
    (OUT / ("phase-" + name + ".json")).write_text(json.dumps(interim, indent=2) + "\n")
    assert equal
    after_witness = witness(root, name + "-after")
    public_after = rpc(root, {"action": "inspect"})
    assert public_before["items"] == public_after["items"] and public_after["same_item"]
    interim.update(
        before_witness=before_witness,
        after_witness=after_witness,
        secret_service_before=public_before,
        secret_service_after=public_after,
        readers_before=readers_before,
        readers_after=no_storage_readers(state["pid"]),
        data_keyring_password_bytes_equal=True,
    )
    if name == "uninstall":
        interim["removed_elfs"] = {
            path: not (root / path).exists()
            for path in (
                "usr/bin/yonik-desktop",
                "usr/lib/yonik/yonik-helper",
                "usr/lib/yonik/yonik-worker",
            )
        }
        assert all(interim["removed_elfs"].values())
    (OUT / ("phase-" + name + ".json")).write_text(json.dumps(interim, indent=2) + "\n")
    completed.append(name)
    phase_file.write_text(json.dumps(completed) + "\n")
    return {"phase": name, "userstate_equal": equal, "witness_before_after": True}


def main() -> None:
    request = json.loads(sys.argv[1])
    state = json.loads((OUT / "active.json").read_text())
    root = Path(state["root"])
    assert str(root).startswith("/var/tmp/codex-myrs/transient/a4-nonempty-")
    assert root.stat().st_uid == os.getuid()
    try:
        if request["action"] == "phase":
            result = phase(state, root, request["phase"])
        else:
            result = rpc(root, request)
            if request["action"] == "capture":
                shutil.copyfile(
                    root / "home/fixture/evidence" / result["file"],
                    OUT / result["file"],
                )
    except Exception as error:
        with (OUT / "failures.jsonl").open("a") as stream:
            stream.write(json.dumps({"request": request, "error": str(error)}) + "\n")
        raise
    with (OUT / "controls.jsonl").open("a") as stream:
        stream.write(json.dumps({"request": request, "result": result}) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
