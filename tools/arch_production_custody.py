"""Production GUI и отдельный Secret Service только внутри disposable QEMU VM."""

from __future__ import annotations

import http.server
import json
import os
import stat
import subprocess
import threading
import time
from pathlib import Path

HOME = Path("/home/builder/production")
OUT = Path("/home/builder/production-evidence")


def session() -> None:
    assert os.getuid() != 0 and Path.home() == HOME
    runtime = HOME / "runtime"
    env = dict(
        os.environ,
        XDG_RUNTIME_DIR=str(runtime),
        XDG_DATA_HOME=str(HOME / "xdg-data"),
        XDG_CONFIG_HOME=str(HOME / "config"),
        DISPLAY=":77",
        QT_QPA_PLATFORM="xcb",
        QT_QUICK_BACKEND="software",
        QT_QUICK_CONTROLS_STYLE="Basic",
    )
    env["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={runtime}/bus"
    env["GNOME_KEYRING_CONTROL"] = str(runtime / "control")
    (HOME / "environment.json").write_text(json.dumps(env))
    conf = HOME / "bus.conf"
    conf.write_text(
        f'<busconfig><type>session</type><listen>unix:path={runtime}/bus</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
    )
    bus = subprocess.Popen(
        ["dbus-daemon", "--nofork", "--config-file", str(conf)],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    xvfb = subprocess.Popen(
        ["Xvfb", ":77", "-screen", "0", "900x900x24", "-nolisten", "tcp"],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    class ProfileFixture(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            body = b"trojan://p24-fixture@profile.example.invalid:443#P24StableProfile"
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *args: object) -> None:
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ProfileFixture)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    (OUT / "source-url.txt").write_text(
        f"http://127.0.0.1:{server.server_port}/fixture"
    )
    tray = None
    daemon = None
    app = None
    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            probe = subprocess.run(
                ["xdotool", "getdisplaygeometry"], env=env, capture_output=True
            )
            if probe.returncode == 0:
                break
            time.sleep(0.1)
        else:
            raise TimeoutError("Xvfb не готов")
        with (OUT / "tray.log").open("w") as log:
            tray = subprocess.Popen(
                [
                    "stalonetray",
                    "--geometry",
                    "1x1+450+0",
                    "--icon-size",
                    "24",
                    "--slot-size",
                    "32",
                ],
                env=env,
                stdout=log,
                stderr=log,
            )
        time.sleep(0.5)
        assert tray.poll() is None, "Tray provider завершился до GUI"
        password = HOME / "password"
        if password.exists():
            metadata = password.lstat()
            assert (
                stat.S_ISREG(metadata.st_mode)
                and metadata.st_uid == os.getuid()
                and metadata.st_mode & 0o777 == 0o600
            )
        else:
            fd = os.open(password, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "wb") as stream:
                stream.write(os.urandom(32).hex().encode())
        with password.open("rb") as stream:
            daemon = subprocess.Popen(
                [
                    "gnome-keyring-daemon",
                    "--foreground",
                    "--unlock",
                    "--components=secrets",
                    "--control-directory",
                    str(runtime / "control"),
                ],
                env=env,
                stdin=stream,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        time.sleep(2)
        with (OUT / "old-empty.log").open("w") as log:
            app = subprocess.Popen(
                ["/usr/bin/yonik-desktop", "--data-dir", str(HOME / "storage")],
                env=env,
                stdout=log,
                stderr=log,
            )
        time.sleep(4)
        subprocess.run(
            [
                "import",
                "-display",
                ":77",
                "-window",
                "root",
                str(OUT / "old-empty.png"),
            ],
            env=env,
            check=True,
        )
        (OUT / "prepared.json").write_text(
            json.dumps(
                {
                    "app_pid": app.pid,
                    "keyring_pid": daemon.pid,
                    "bus_pid": bus.pid,
                    "app_args": [
                        "/usr/bin/yonik-desktop",
                        "--data-dir",
                        str(HOME / "storage"),
                    ],
                    "production_custody": True,
                    "ephemeral_key": False,
                }
            )
        )
        subprocess.run(
            ["python", str(Path(__file__).with_name("production-route.py"))],
            env=env,
            check=True,
        )
        assert json.loads((OUT / "result.json").read_text())["status"] == "PASS"
    finally:
        server.shutdown()
        server.server_close()
        for process in (app, daemon, bus, tray, xvfb):
            if process is not None and process.poll() is None:
                process.terminate()
                process.wait(timeout=15)


def main() -> None:
    assert Path("/sys/class/dmi/id/sys_vendor").read_text().strip() == "QEMU"
    if os.getuid() != 0:
        session()
        return
    OUT.mkdir()
    subprocess.run(
        [
            "install",
            "-d",
            "-m700",
            "-o",
            "builder",
            "-g",
            "builder",
            str(HOME),
            str(HOME / "runtime"),
            str(HOME / "storage"),
        ],
        check=True,
    )
    subprocess.run(["chown", "builder:builder", str(OUT)], check=True)
    subprocess.run(
        ["pacman", "-U", "--noconfirm", "/home/builder/old.pkg.tar.zst"], check=True
    )
    state = subprocess.run(
        ["systemctl", "is-active", "yonik-helper.service"],
        capture_output=True,
        text=True,
    )
    assert (state.returncode, state.stdout.strip()) == (3, "inactive")
    subprocess.run(["nft", "add", "table", "inet", "yonik_custody_fixture"], check=True)
    subprocess.run(["ip", "link", "add", "yonik-custody", "type", "dummy"], check=True)

    def foreign() -> dict[str, str]:
        return {
            "nft": subprocess.check_output(
                ["nft", "list", "table", "inet", "yonik_custody_fixture"], text=True
            ),
            "link": subprocess.check_output(
                ["ip", "-j", "link", "show", "yonik-custody"], text=True
            ),
        }

    subprocess.run(["udevadm", "settle", "--timeout=15"], check=True)
    before = foreign()
    (OUT / "foreign-before.json").write_text(json.dumps(before))
    try:
        subprocess.run(
            [
                "runuser",
                "-u",
                "builder",
                "--",
                "env",
                f"HOME={HOME}",
                "python",
                __file__,
            ],
            check=True,
        )
        after = foreign()
        (OUT / "foreign-after.json").write_text(json.dumps(after))
        assert after == before
    finally:
        subprocess.run(["ip", "link", "del", "yonik-custody"], check=True)
        subprocess.run(
            ["nft", "delete", "table", "inet", "yonik_custody_fixture"], check=True
        )


if __name__ == "__main__":
    main()
