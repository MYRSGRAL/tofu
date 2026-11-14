"""Управление только own-prefix GUI через private Unix QVNC/control sockets."""

from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import time
from pathlib import Path

from a4_rfb import Rfb

OUT = Path(__file__).resolve().parents[1] / "docs/evidence/a4-mtree4/gui-run"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "action",
        choices=[
            "capture",
            "click",
            "key",
            "text",
            "status",
            "restart-gui",
            "finish",
            "probe",
        ],
    )
    parser.add_argument("--role", choices=["gui", "provider"], default="gui")
    parser.add_argument("--x", type=int, default=0)
    parser.add_argument("--y", type=int, default=0)
    parser.add_argument("--key", type=int, default=0)
    parser.add_argument("--file", type=Path)
    parser.add_argument("--label", default="capture")
    args = parser.parse_args()
    state = json.loads((OUT / "active.json").read_text())
    root = Path(state["root"])
    assert (
        str(root).startswith("/var/tmp/codex-myrs/transient/a4-gui-")
        and root.stat().st_uid == os.getuid()
    )
    response = {}
    if args.action in ("status", "restart-gui", "finish"):
        with socket.socket(socket.AF_UNIX) as connection:
            connection.connect(str(root / "home/fixture/runtime/control"))
            connection.sendall(json.dumps({"action": args.action}).encode())
            response = json.loads(connection.recv(4096))
    elif args.action == "probe":
        address = "unix:path=" + str(root / "home/fixture/runtime/bus")

        def bus(
            destination: str, path: str, interface: str, method: str, *parameters: str
        ) -> dict:
            process = subprocess.run(
                [
                    "busctl",
                    "--address=" + address,
                    "--json=short",
                    "call",
                    destination,
                    path,
                    interface,
                    method,
                    *parameters,
                ],
                capture_output=True,
                text=True,
            )
            return {
                "exit": process.returncode,
                "output": process.stdout,
                "stderr": process.stderr,
            }

        response["owner"] = bus(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            "s",
            "org.freedesktop.secrets",
        )
        response["items"] = bus(
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
            "SearchItems",
            "a{ss}",
            "3",
            "application",
            "cloud.highpriority.tofu",
            "purpose",
            "subscription-master-key",
            "schema",
            "1",
        )
    else:
        display = Rfb(root / ("home/fixture/runtime/vnc-" + args.role))
        try:
            if args.action == "capture":
                display.capture(OUT / (args.label + ".png"))
            elif args.action == "click":
                display.click(args.x, args.y)
            elif args.action == "key":
                display.key(args.key)
            elif args.action == "text":
                assert args.file is not None
                display.text(args.file.read_text())
            time.sleep(0.2)
        finally:
            display.close()
    record = {
        "action": args.action,
        "role": args.role,
        "label": args.label,
        "x": args.x,
        "y": args.y,
        "key": args.key,
        "response": response,
    }
    with (OUT / "controls.jsonl").open("a") as log:
        log.write(json.dumps(record) + "\n")
    if response:
        os.write(1, (json.dumps(response) + "\n").encode())


if __name__ == "__main__":
    main()
