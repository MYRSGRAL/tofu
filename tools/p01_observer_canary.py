"""Короткий контроль private observer на actual GUI; не acceptance P01."""

import json
import os
import signal
import socket
import subprocess
import time
from pathlib import Path


def main() -> None:
    home = Path("/home/fixture")
    if Path.home() != home or os.getuid() == 0:
        raise RuntimeError("Только отдельный fixture user в credential VM")
    env = dict(
        os.environ,
        XDG_RUNTIME_DIR=str(home / "runtime"),
        XDG_DATA_HOME=str(home / "data"),
        XDG_CONFIG_HOME=str(home / "config"),
        DBUS_SESSION_BUS_ADDRESS=f"unix:path={home}/runtime/bus",
        DISPLAY=":99",
        QT_QPA_PLATFORM="xcb",
        QT_QUICK_BACKEND="software",
        YONIK_CREDENTIAL_VM="disposable-v1",
    )
    client, child = socket.socketpair()
    client.settimeout(10)
    env["YONIK_P01_CONTROL_FD"] = str(child.fileno())
    observations: list[dict] = []
    with (home / "p01-gui.log").open("wb") as log:
        process = subprocess.Popen(
            ["/usr/local/bin/yonik-desktop", "--data-dir", str(home / "p01-storage")],
            env=env,
            pass_fds=(child.fileno(),),
            stdout=log,
            stderr=log,
            start_new_session=True,
        )
        child.close()
        stream = client.makefile("rb")

        def command(value: str) -> dict:
            client.sendall(value.encode() + b"\n")
            reply = json.loads(stream.readline(4096))
            if reply["command"] != value or not reply["ok"]:
                raise RuntimeError(f"Control rejected: {value}")
            observations.append(reply)
            return reply

        try:
            deadline = time.monotonic() + 40
            while True:
                state = command("Snapshot")
                if state.get("ready"):
                    break
                if process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("GUI not ready")
                time.sleep(0.5)
            if state.get("can_stop") or state.get("status") != "Disconnected":
                raise RuntimeError("Canary требует no-connect")
            command("Open")
            time.sleep(1)
            before = command("Snapshot")
            command("VisibleFrameCanary")
            time.sleep(1)
            after = command("Snapshot")
            if not after["visible"] or after["frames"] <= before["frames"]:
                raise RuntimeError("Frame oracle нечувствителен")
            command("Hide")
            time.sleep(1)
            hidden = command("Snapshot")
            time.sleep(2)
            quiet = command("Snapshot")
            if (
                hidden["visible"]
                or quiet["visible"]
                or quiet["frames"] != hidden["frames"]
            ):
                raise RuntimeError("Hidden oracle control failed")
            command("Quit")
            if process.wait(timeout=15) != 0:
                raise RuntimeError("GUI quit failed")
            print(
                json.dumps(
                    {"kind": "short-frame-canary-NOT-P01", "observations": observations}
                )
            )
        finally:
            client.close()
            stream.close()
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)


if __name__ == "__main__":
    main()
