"""Узкий controller own session: X11 input/RPC и связанные package data phases."""

from __future__ import annotations

import json
import os
import shutil
import socket
import sys
from pathlib import Path

import a4_candidate_lifecycle as c
from a4_userdata_prefix import sha
from a4_userdata_session import NEW

BASE = Path(__file__).resolve().parents[1]
OUT = BASE / "docs/evidence/a4-userdata-142/session7"


def rpc(root: Path, request: dict) -> dict:
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(90)
        connection.connect(str(root / "home/fixture/runtime/a4-control"))
        connection.sendall(json.dumps(request).encode())
        data = bytearray()
        while True:
            part = connection.recv(65536)
            if not part:
                break
            data.extend(part)
    result = json.loads(data)
    if "error" in result:
        raise RuntimeError(result["error"])
    return result


def snapshot(root: Path) -> dict:
    result = {}
    for path in (root / "home/fixture/storage").rglob("*"):
        if path.is_file():
            info = path.stat()
            result[str(path.relative_to(root / "home/fixture"))] = {
                "sha256": sha(path),
                "mode": oct(info.st_mode & 0o777),
                "uid": info.st_uid,
                "size": info.st_size,
            }
    return result


def main() -> None:
    request = json.loads(sys.argv[1])
    state = json.loads((OUT / "active.json").read_text())
    root = Path(state["root"])
    assert (
        str(root).startswith("/var/tmp/codex-myrs/transient/a4-userdata-")
        and root.stat().st_uid == os.getuid()
    )
    if request["action"] == "phase":
        assert rpc(root, {"action": "status"})["gui_exit"] == 0
        phase = request["phase"]
        assert phase in ("upgrade", "rollback", "uninstall")
        phasefile = OUT / "package-phases.json"
        phases = json.loads(phasefile.read_text()) if phasefile.exists() else []
        assert phase == ["upgrade", "rollback", "uninstall"][len(phases)]
        before = snapshot(root)
        public_before = rpc(root, {"action": "inspect"})
        # Сравниваются opaque bytes keyring, без экспорта SHA/ключа/его hash.
        keyrings = {
            str(p.relative_to(root)): p.read_bytes()
            for p in (root / "home/fixture/data").rglob("*.keyring")
        }
        package = NEW if phase == "upgrade" else c.OLD
        assert sha(package) == (
            "26a013747e4c1dbb95e7290cd37b981f5dc7ef891a8865897f09fc45bacd0237"
            if phase == "upgrade"
            else "e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0"
        )
        c.OUT = OUT
        c.command(
            [*state["pacman"], "-R", "-dd", "yonik"]
            if phase == "uninstall"
            else [*state["pacman"], "-U", "-dd", str(package)]
        )
        after = snapshot(root)
        public_after = rpc(root, {"action": "inspect"})
        assert (
            before == after
            and public_before["items"] == public_after["items"]
            and public_after["same_item"]
        )
        assert all(
            (root / name).read_bytes() == data for name, data in keyrings.items()
        )
        result = {
            "phase": phase,
            "data_equal": True,
            "keyring_bytes_equal": True,
            "item_equal": True,
            "before": before,
            "after": after,
            "secret_service": public_after,
        }
        if phase == "uninstall":
            assert not (root / "usr/bin/yonik-desktop").exists()
            result["desktop_removed"] = True
        (OUT / ("phase-" + phase + ".json")).write_text(
            json.dumps(result, indent=2) + "\n"
        )
        phases.append(phase)
        phasefile.write_text(json.dumps(phases) + "\n")
    elif request["action"] == "supply-witness":
        binding = json.loads(
            (OUT.parent / "witness-attempt2/witness-build-binding.json").read_text()
        )
        assert binding["exit"] == 0
        source = BASE / "local/validation/a4-userdata-142/a4_prefix_witness"
        assert sha(source) == binding["elf_sha256"]
        target = root / "usr/local/bin/a4_prefix_witness"
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        target.chmod(0o755)
        result = {"elf_sha256": sha(target)}
    else:
        result = rpc(root, request)
        if request["action"] == "capture":
            shutil.copyfile(
                root / "home/fixture/evidence" / result["file"], OUT / result["file"]
            )
    with (OUT / "controls.jsonl").open("a") as log:
        log.write(json.dumps({"request": request, "result": result}) + "\n")
    os.write(1, (json.dumps(result) + "\n").encode())


if __name__ == "__main__":
    main()
