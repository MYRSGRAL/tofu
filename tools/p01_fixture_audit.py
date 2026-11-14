"""Ограниченный read-only аудит трёх собственных credential VM P01."""

import hashlib
import json
import subprocess
import time
from pathlib import Path

ROOT = Path("/home/myrs/projects/yonik/local/validation/p01-178")
BINARY = Path(
    "/home/myrs/projects/yonik/local/artifacts/p01-178/yonik-desktop-observer"
)


def main() -> None:
    expected = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    deadline = time.monotonic() + 7200
    for index in range(1, 4):
        vmroot = Path("/var/tmp/codex-myrs/transient") / (
            "p01-178-credential-vm" if index == 1 else f"p01-178-vm-{index}"
        )
        connection = vmroot / "connection.json"
        output = ROOT / f"repeat-{index}"
        # Аудит после setup, когда main controller уже запустил измеритель.
        while not (output / "host-context.jsonl").exists():
            if time.monotonic() > deadline:
                raise TimeoutError("Fixture audit deadline")
            time.sleep(5)
        ssh = json.loads(connection.read_text())["ssh"]
        digest = subprocess.run(
            ssh + ["sha256sum /usr/local/bin/yonik-desktop"],
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        ).stdout.split()[0]
        if digest != expected:
            raise RuntimeError("Guest ELF не совпадает с frozen observer ELF")
        custody = subprocess.run(
            ssh + ["runuser -u fixture -- python /home/fixture/format4_gui_guest.py"],
            input=b'{"action":"inspect"}',
            check=True,
            capture_output=True,
            timeout=30,
        )
        value = json.loads(custody.stdout)
        if not value["owner_pid_matches"]:
            raise RuntimeError("Secret Service custody mismatch")
        (output / "fixture-audit.json").write_text(
            json.dumps(
                {
                    "guest_elf_sha256": digest,
                    "host_elf_matches": True,
                    "secret_service": value,
                    "audit_epoch": time.time(),
                },
                indent=2,
            )
            + "\n"
        )
    (ROOT / "fixture-audit-terminal.json").write_text('{"state":"COMPLETE"}\n')


if __name__ == "__main__":
    main()
