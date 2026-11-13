"""Host CLI: только собственная VM; публичные результаты и снимки её экрана."""

import fcntl
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path("/var/tmp/codex-myrs/transient/secret-format4-gui")
EVIDENCE = Path("local/validation/secret-format4-gui")


def main() -> None:
    fd = os.open(ROOT, os.O_RDONLY | os.O_DIRECTORY)
    fcntl.flock(fd, fcntl.LOCK_SH)
    ssh = json.loads((ROOT / "connection.json").read_text())["ssh"]
    request = json.loads(sys.argv[1])
    # Контракт CLI не принимает payload/ключ: text читает fixture только в guest.
    with (EVIDENCE / "operations.jsonl").open("a") as stream:
        stream.write(json.dumps(request) + "\n")
    if request["action"] == "upload":
        for source, destination in [
            (Path("tools/format4_gui_guest.py"), "/home/fixture/format4_gui_guest.py"),
            (EVIDENCE / "yonik-desktop", "/usr/local/bin/yonik-desktop"),
            (
                EVIDENCE / "format4_custody_witness",
                "/usr/local/bin/format4_custody_witness",
            ),
        ]:
            subprocess.run(
                ssh + ["cat > " + destination + "; chmod 755 " + destination],
                input=source.read_bytes(),
                check=True,
                timeout=60,
            )
        return
    response = subprocess.run(
        ssh + ["runuser -u fixture -- python /home/fixture/format4_gui_guest.py"],
        input=json.dumps(request).encode(),
        capture_output=True,
        timeout=40,
    )
    if response.returncode:
        sys.stderr.buffer.write(response.stderr)
        raise RuntimeError("Guest operation failed")
    if request["action"] == "capture":
        (EVIDENCE / request["name"]).write_bytes(response.stdout)
    else:
        value = json.loads(response.stdout)
        with (EVIDENCE / "observations.jsonl").open("a") as stream:
            stream.write(
                json.dumps(
                    {
                        "action": request["action"],
                        "stage": request.get("stage"),
                        "result": value,
                    }
                )
                + "\n"
            )
        print(json.dumps(value))
    os.close(fd)


if __name__ == "__main__":
    main()
