"""Внешние X11 input для normal GUI в собственной одноразовой VM."""

import json
import os
import signal
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

HOME = Path("/home/fixture")
RUNTIME = HOME / "runtime"
STORAGE = HOME / "storage"


def main() -> None:
    assert Path("/sys/class/dmi/id/sys_vendor").read_text().strip() == "QEMU"
    assert os.getuid() != 0 and Path.home() == HOME
    env = dict(
        os.environ,
        XDG_RUNTIME_DIR=str(RUNTIME),
        XDG_DATA_HOME=str(HOME / "data"),
        XDG_CONFIG_HOME=str(HOME / "config"),
        DBUS_SESSION_BUS_ADDRESS=f"unix:path={RUNTIME}/bus",
        DISPLAY=":99",
        QT_QPA_PLATFORM="xcb",
        QT_QUICK_BACKEND="software",
        YONIK_CREDENTIAL_VM="disposable-v1",
    )
    request = json.load(sys.stdin)

    def call(args: list[str], **kwargs: object) -> bytes:
        return subprocess.run(
            args, env=env, check=True, capture_output=True, timeout=30, **kwargs
        ).stdout

    def spawn(args: list[str], stdin: object = subprocess.DEVNULL) -> int:
        return subprocess.Popen(
            args,
            env=env,
            stdin=stdin,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        ).pid

    def bus(
        destination: str, path: str, interface: str, method: str, *args: str
    ) -> object:
        return json.loads(
            call(
                [
                    "busctl",
                    "--address=" + env["DBUS_SESSION_BUS_ADDRESS"],
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

    def names() -> object:
        return bus(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListNames",
        )

    action = request["action"]
    result = {}
    if action == "init":
        STORAGE.mkdir(mode=0o700)
        (STORAGE / "copies").mkdir(mode=0o700)
        for name in ["text", "file"]:
            (STORAGE / (name + ".txt")).write_text(
                f"trojan://fixture-{name}@example.invalid:443#{name}"
            )
        config = HOME / "bus.conf"
        config.write_text(
            f"<busconfig><type>session</type><listen>unix:path={RUNTIME}/bus</listen>"
            '<auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/>'
            '<allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
        )
        pids = {"bus": spawn(["dbus-daemon", "--nofork", "--config-file", str(config)])}
        time.sleep(0.5)
        assert "org.freedesktop.secrets" not in names()[0]
        password = HOME / "password"
        fd = os.open(password, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(os.urandom(32).hex().encode())
        with password.open("rb") as stream:
            pids["keyring"] = spawn(
                [
                    "gnome-keyring-daemon",
                    "--foreground",
                    "--unlock",
                    "--components=secrets",
                    "--control-directory",
                    str(RUNTIME / "control"),
                ],
                stream,
            )
        pids["xvfb"] = spawn(
            ["Xvfb", ":99", "-screen", "0", "1024x900x24", "-nolisten", "tcp"]
        )
        time.sleep(1)
        pids["wm"] = spawn(["openbox"])
        pids["tray"] = spawn(["tint2"])
        (HOME / "pids.json").write_text(json.dumps(pids))
        result = {"no_activation_before": True}
    elif action == "launch":
        pid = spawn(["/usr/local/bin/yonik-desktop", "--data-dir", str(STORAGE)])
        (HOME / "gui.pid").write_text(str(pid))
        result = {"pid": pid}
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
        call(["xdotool", "key", request["key"]])
    elif action in ["path", "basename"]:
        assert request["name"] in ["file.txt", "backup3.sqlite"]
        call(
            ["xdotool", "type", "--clearmodifiers", "--file", "-"],
            input=(
                str(STORAGE / request["name"]) if action == "path" else request["name"]
            ).encode(),
        )
    elif action == "position":
        pid = (HOME / "gui.pid").read_text().strip()
        window = (
            call(["xdotool", "search", "--onlyvisible", "--pid", pid])
            .decode()
            .splitlines()[0]
        )
        call(["xdotool", "windowmove", window, "150", "20"])
        call(["xdotool", "windowsize", window, "600", "820"])
    elif action == "text":
        call(
            ["xdotool", "type", "--clearmodifiers", "--file", "-"],
            input=(STORAGE / "text.txt").read_bytes(),
        )
    elif action == "capture":
        sys.stdout.buffer.write(call(["import", "-window", "root", "png:-"]))
        return
    elif action == "inspect":
        owner = bus(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            "s",
            "org.freedesktop.secrets",
        )[0]
        owner_pid = bus(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetConnectionUnixProcessID",
            "s",
            owner,
        )[0]
        assert owner_pid == json.loads((HOME / "pids.json").read_text())["keyring"]
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
        result = {
            "owner_pid_matches": True,
            "items": items,
            "count": sum(map(len, items)),
        }
        for name in ["subscriptions.sqlite", "backup3.sqlite"]:
            path = (
                STORAGE / name
                if name == "subscriptions.sqlite"
                else STORAGE / "copies" / name
            )
            if path.exists():
                with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True) as db:
                    result[name] = {
                        "version": db.execute("PRAGMA user_version").fetchone()[0],
                        "integrity": db.execute("PRAGMA integrity_check").fetchone()[0]
                        == "ok",
                    }
    elif action == "witness":
        result = json.loads(
            call(["/usr/local/bin/format4_custody_witness", str(STORAGE)])
        )
    elif action == "closed":
        pid = int((HOME / "gui.pid").read_text())
        result = {"closed": not Path(f"/proc/{pid}/exe").exists()}
        assert result["closed"]
    elif action == "finish":
        pids = json.loads((HOME / "pids.json").read_text())
        os.kill(pids["keyring"], signal.SIGTERM)
        for _ in range(100):
            if "org.freedesktop.secrets" not in names()[0]:
                break
            time.sleep(0.05)
        else:
            raise RuntimeError("Secret Service owner не исчез")
        result = {"name_disappeared": True}
        for name in ["tray", "wm", "xvfb", "bus"]:
            os.kill(pids[name], signal.SIGTERM)
    else:
        raise ValueError("Неизвестное действие")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
