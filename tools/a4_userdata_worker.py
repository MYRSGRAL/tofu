"""Normal X11 GUI + настоящий private gnome-keyring; RPC только fixture input."""

from __future__ import annotations

import json
import os
import re
import secrets
import socket
import sqlite3
import subprocess
import time
from pathlib import Path

HOME = Path("/home/fixture")
DATA = HOME / "storage"
RUNTIME = HOME / "runtime"
EVIDENCE = HOME / "evidence"


def main() -> None:
    os.umask(0o077)
    assert os.getuid() == os.geteuid() != 0
    assert (
        Path.home() == HOME
        and os.environ["DBUS_SESSION_BUS_ADDRESS"]
        == "unix:path=/home/fixture/runtime/bus"
    )
    host = json.loads(os.environ["A4_HOST_NAMESPACES"])
    own = {
        name: os.readlink("/proc/self/ns/" + name)
        for name in ("user", "mnt", "pid", "net")
    }
    assert all(own[name] != host[name] for name in own)
    children = []
    logs = []
    gui = None
    sequence = 0
    expected_item = None
    nonce = secrets.token_hex(32)

    def log(row: dict) -> None:
        with (EVIDENCE / "operations.jsonl").open("a") as stream:
            stream.write(json.dumps(row) + "\n")

    def spawn(
        name: str, args: list[str], password: bytes | None = None
    ) -> subprocess.Popen:
        output = (EVIDENCE / (name + ".log")).open("ab")
        logs.append(output)
        process = subprocess.Popen(
            args,
            stdin=subprocess.PIPE if password is not None else subprocess.DEVNULL,
            stdout=output,
            stderr=output,
        )
        if password is not None:
            assert process.stdin is not None
            process.stdin.write(password)
            process.stdin.close()
        children.append(process)
        log({"spawn": name, "pid": process.pid, "uid": os.getuid(), "argv": args})
        return process

    def call(args: list[str], payload: bytes | None = None) -> bytes:
        process = subprocess.run(args, input=payload, capture_output=True, timeout=30)
        # Text payload/password/ключи никогда не входят в command log.
        log({"argv": args, "exit": process.returncode})
        if process.returncode:
            with (EVIDENCE / "command-errors.jsonl").open("a") as stream:
                stream.write(
                    json.dumps(
                        {
                            "argv": args,
                            "exit": process.returncode,
                            "stderr": process.stderr.decode(errors="replace"),
                        }
                    )
                    + "\n"
                )
            raise RuntimeError("Fixture command отказал: " + args[0])
        return process.stdout

    def bus(
        destination: str, path: str, interface: str, method: str, *args: str
    ) -> object:
        return json.loads(
            call(
                [
                    "busctl",
                    "--address=" + os.environ["DBUS_SESSION_BUS_ADDRESS"],
                    "--json=short",
                    "call",
                    destination,
                    path,
                    interface,
                    method,
                    *args,
                ]
            )
        )["data"]

    def inspect() -> dict:
        nonlocal expected_item
        owner = bus(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            "s",
            "org.freedesktop.secrets",
        )[0]
        pid = bus(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetConnectionUnixProcessID",
            "s",
            owner,
        )[0]
        uid = bus(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetConnectionUnixUser",
            "s",
            owner,
        )[0]
        assert pid == provider.pid and uid == os.getuid()
        items = bus(
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
        if len(items[0]) == 1 and not items[1]:
            if expected_item is None:
                expected_item = items[0][0]
            assert items[0][0] == expected_item
        result = {
            "owner": owner,
            "provider_pid": pid,
            "uid": uid,
            "items": items,
            "same_item": expected_item is not None and items == [[expected_item], []],
        }
        for label, path in [
            ("live", DATA / "subscriptions.sqlite"),
            ("backup", DATA / "copies/backup3.sqlite"),
        ]:
            if path.exists():
                with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True) as db:
                    result[label] = {
                        "schema": db.execute("PRAGMA user_version").fetchone()[0],
                        "integrity_ok": db.execute("PRAGMA integrity_check").fetchone()[
                            0
                        ]
                        == "ok",
                    }
        return result

    try:
        bus_process = spawn(
            "bus", ["dbus-daemon", "--nofork", "--config-file=/home/fixture/bus.conf"]
        )
        for _ in range(100):
            if (RUNTIME / "bus").exists():
                break
            assert bus_process.poll() is None
            time.sleep(0.05)
        password = secrets.token_hex(32).encode()
        (HOME / "keyring-password").write_bytes(password)
        provider = spawn(
            "provider",
            [
                "gnome-keyring-daemon",
                "--foreground",
                "--unlock",
                "--components=secrets",
                "--control-directory",
                str(RUNTIME / "control-keyring"),
            ],
            password,
        )
        del password
        for _ in range(100):
            try:
                inspect()
                break
            except (RuntimeError, AssertionError):
                assert provider.poll() is None
                time.sleep(0.05)
        else:
            raise RuntimeError("Private Secret Service не готов")
        Path("/tmp/.X11-unix").mkdir(mode=0o1777, exist_ok=True)
        xvfb = spawn(
            "xvfb",
            ["Xvfb", ":99", "-screen", "0", "1024x900x24", "-nolisten", "tcp", "-ac"],
        )
        for _ in range(100):
            ready = subprocess.run(
                ["xdotool", "getdisplaygeometry"], capture_output=True
            )
            if ready.returncode == 0:
                break
            assert xvfb.poll() is None
            time.sleep(0.05)
        else:
            raise RuntimeError("Own X display не готов")
        wm = spawn("wm", ["openbox"])
        tray = spawn("tray", ["tint2"])
        time.sleep(1)
        assert wm.poll() is None and tray.poll() is None
        control = socket.socket(socket.AF_UNIX)
        control.bind(str(RUNTIME / "a4-control"))
        control.listen()
        log(
            {
                "ready": True,
                "namespace": own,
                "root_identity": {
                    "dev": os.stat("/").st_dev,
                    "ino": os.stat("/").st_ino,
                },
            }
        )
        while True:
            connection, _ = control.accept()
            with connection:
                request = json.loads(connection.recv(8192))
                action = request["action"]
                assert re.fullmatch(
                    r"[a-zA-Z0-9_-]{1,64}", request.get("stage", "step")
                )
                result = {}
                try:
                    if action == "launch":
                        assert gui is None or gui.poll() is not None
                        sequence += 1
                        gui = spawn(
                            "gui-" + str(sequence),
                            ["/usr/bin/yonik-desktop", "--data-dir", str(DATA)],
                        )
                        result = {"pid": gui.pid}
                    elif action == "status":
                        result = {
                            "gui_exit": None if gui is None else gui.poll(),
                            "provider_exit": provider.poll(),
                        }
                    elif action == "position":
                        assert gui is not None
                        windows = (
                            call(
                                [
                                    "xdotool",
                                    "search",
                                    "--onlyvisible",
                                    "--pid",
                                    str(gui.pid),
                                ]
                            )
                            .decode()
                            .splitlines()
                        )
                        assert windows
                        call(["xdotool", "windowmove", windows[0], "150", "20"])
                        call(["xdotool", "windowsize", windows[0], "600", "820"])
                    elif action == "click":
                        call(
                            [
                                "xdotool",
                                "mousemove",
                                str(request["x"]),
                                str(request["y"]),
                                "click",
                                str(request.get("button", 1)),
                            ]
                        )
                    elif action == "key":
                        call(["xdotool", "key", "--clearmodifiers", request["key"]])
                    elif action == "text":
                        call(
                            ["xdotool", "type", "--clearmodifiers", "--file", "-"],
                            (DATA / "text.txt").read_bytes(),
                        )
                    elif action == "path":
                        assert request["name"] in ("file.txt", "backup3.sqlite")
                        path = (
                            DATA / request["name"]
                            if request["name"] == "file.txt"
                            else DATA / "copies/backup3.sqlite"
                        )
                        call(
                            ["xdotool", "type", "--clearmodifiers", "--file", "-"],
                            str(path).encode(),
                        )
                    elif action == "capture":
                        name = request["stage"] + ".png"
                        call(["import", "-window", "root", str(EVIDENCE / name)])
                        result = {"file": name}
                    elif action == "inspect":
                        result = inspect()
                    elif action == "witness":
                        state = inspect()
                        assert state["same_item"]
                        guard = {
                            "source": "14229843f9a11bf98ec4f5992df00f3247102ea2",
                            "uid": os.getuid(),
                            "nonce": nonce,
                            "namespace": own,
                            "host_namespace": host,
                            "provider_pid": provider.pid,
                            "provider_owner": state["owner"],
                            "item_path": expected_item,
                            "root": {
                                "dev": os.stat("/").st_dev,
                                "ino": os.stat("/").st_ino,
                            },
                            "data": {
                                "dev": DATA.stat().st_dev,
                                "ino": DATA.stat().st_ino,
                            },
                        }
                        guard_path = RUNTIME / "witness-guard.json"
                        guard_path.write_text(json.dumps(guard))
                        guard_path.chmod(0o600)
                        env = dict(os.environ, A4_WITNESS_NONCE=nonce)
                        run = subprocess.run(
                            ["/usr/local/bin/a4_prefix_witness"],
                            env=env,
                            capture_output=True,
                            text=True,
                            timeout=60,
                        )
                        result = {
                            "exit": run.returncode,
                            "stdout": run.stdout,
                            "stderr": run.stderr,
                        }
                        assert run.returncode == 0, result
                    elif action == "closed":
                        assert gui is not None and gui.poll() == 0
                        result = {
                            "closed": True,
                            "pid": gui.pid,
                            "exit": gui.returncode,
                        }
                    elif action == "finish":
                        assert gui is None or gui.poll() is not None
                        result = {"finishing": True}
                        connection.sendall(json.dumps(result).encode())
                        log({"request": request, "result": result})
                        break
                    else:
                        raise ValueError("Недопустимая fixture операция")
                except Exception as error:
                    # Ошибки API содержат только public gate messages; GetSecret не вызывается fixture.
                    result = {"error": str(error)}
                log({"request": request, "result": result})
                connection.sendall(json.dumps(result).encode())
    finally:
        for process in reversed(children):
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        (EVIDENCE / "exits.json").write_text(
            json.dumps([{"pid": p.pid, "exit": p.returncode} for p in children]) + "\n"
        )
        for stream in logs:
            stream.close()


if __name__ == "__main__":
    main()
