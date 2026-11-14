"""Управляемый private-bus/QVNC supervisor внутри own disposable prefix."""

from __future__ import annotations

import json
import os
import select
import socket
import subprocess
import threading
import time
from pathlib import Path

HOME = Path("/home/fixture")
RUNTIME = HOME / "runtime"


def bridge(name: str, port: int) -> None:
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(str(RUNTIME / name))
    listener.listen()
    while True:
        client, _ = listener.accept()
        try:
            remote = socket.create_connection(("127.0.0.1", port), timeout=5)
            while True:
                ready, _, _ = select.select([client, remote], [], [], 60)
                if not ready:
                    break
                ended = False
                for stream in ready:
                    data = stream.recv(1024 * 1024)
                    if not data:
                        ended = True
                        break
                    (remote if stream is client else client).sendall(data)
                if ended:
                    break
            remote.close()
        except OSError:
            pass  # QVNC появляется только при запуске соответствующего GUI.
        finally:
            client.close()


def main() -> None:
    assert os.getuid() != 0
    children = []
    logs = []

    def spawn(
        name: str, args: list[str], platform: str | None = None
    ) -> subprocess.Popen:
        log = (HOME / (name + ".log")).open("ab")
        logs.append(log)
        env = dict(os.environ)
        if platform:
            env["QT_QPA_PLATFORM"] = platform
        process = subprocess.Popen(args, env=env, stdout=log, stderr=log)
        children.append(process)
        with (HOME / "pids.jsonl").open("a") as stream:
            stream.write(
                json.dumps(
                    {"name": name, "pid": process.pid, "uid": os.getuid(), "argv": args}
                )
                + "\n"
            )
        return process

    try:
        spawn(
            "bus",
            [
                "/usr/bin/dbus-daemon",
                "--nofork",
                "--config-file=/home/fixture/bus.conf",
            ],
        )
        for _ in range(50):
            if (RUNTIME / "bus").exists():
                break
            time.sleep(0.1)
        provider = spawn(
            "provider", ["/usr/bin/ksecretd"], "vnc:size=1024x900:port=5901"
        )
        gui = spawn(
            "gui",
            ["/usr/bin/yonik-desktop", "--data-dir", "/home/fixture/storage"],
            "vnc:size=1024x900:port=5900",
        )
        for name, port in [("vnc-gui", 5900), ("vnc-provider", 5901)]:
            threading.Thread(target=bridge, args=(name, port), daemon=True).start()
        control = socket.socket(socket.AF_UNIX)
        control.bind(str(RUNTIME / "control"))
        control.listen()
        while True:
            connection, _ = control.accept()
            with connection:
                request = json.loads(connection.recv(4096))
                if request["action"] == "status":
                    response = {
                        "gui_exit": gui.poll(),
                        "provider_exit": provider.poll(),
                    }
                elif request["action"] == "restart-gui":
                    if gui.poll() is None:
                        raise RuntimeError("Предыдущий GUI ещё работает")
                    gui = spawn(
                        "gui-reopen",
                        [
                            "/usr/bin/yonik-desktop",
                            "--data-dir",
                            "/home/fixture/storage",
                        ],
                        "vnc:size=1024x900:port=5900",
                    )
                    response = {"pid": gui.pid}
                elif request["action"] == "finish":
                    connection.sendall(b'{"finishing":true}')
                    break
                else:
                    response = {"error": "Недопустимая операция"}
                connection.sendall(json.dumps(response).encode())
    finally:
        for process in reversed(children):
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        (HOME / "exits.json").write_text(
            json.dumps([{"pid": p.pid, "exit": p.returncode} for p in children])
        )
        for log in logs:
            log.close()


if __name__ == "__main__":
    main()
