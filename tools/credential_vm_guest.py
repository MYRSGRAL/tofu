"""Guest-only controller: отдельный пользователь, bus без autolaunch, свой keyring."""

import hashlib
import json
import os
import subprocess
import time
from pathlib import Path


def main() -> None:
    assert Path("/sys/class/dmi/id/sys_vendor").read_text().strip() == "QEMU"
    assert os.getuid() != 0
    home = Path("/home/fixture")
    assert Path.home() == home
    runtime = home / "runtime"
    env = dict(
        os.environ,
        XDG_RUNTIME_DIR=str(runtime),
        XDG_DATA_HOME=str(home / "data"),
        XDG_CONFIG_HOME=str(home / "config"),
        YONIK_CREDENTIAL_VM="disposable-v1",
    )
    config = home / "bus.conf"
    config.write_text(
        f"<busconfig><type>session</type><listen>unix:path={runtime}/bus</listen>"
        '<auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/>'
        '<allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
    )
    bus = subprocess.Popen(
        ["dbus-daemon", "--nofork", "--config-file", str(config)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    daemon = None
    env["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={runtime}/bus"
    env["GNOME_KEYRING_CONTROL"] = str(runtime / "control")
    results = {
        "packages": subprocess.run(
            ["pacman", "-Q", "gnome-keyring", "dbus"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.splitlines()
    }

    def call(args: list[str], **kwargs: object) -> str:
        return (
            subprocess.run(
                args, env=env, check=True, capture_output=True, timeout=30, **kwargs
            )
            .stdout.decode()
            .strip()
        )

    def names() -> str:
        return call(
            [
                "dbus-send",
                "--session",
                "--print-reply",
                "--dest=org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus.ListNames",
            ]
        )

    def start() -> subprocess.Popen:
        # Пароль создаётся только в VM, не попадает в argv/stdout/evidence.
        password = home / "password"
        if not password.exists():
            fd = os.open(password, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "wb") as stream:
                stream.write(os.urandom(32).hex().encode())
        with password.open("rb") as stream:
            process = subprocess.Popen(
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
        unlocked = False
        for _ in range(100):
            if "org.freedesktop.secrets" in names():
                if not unlocked:
                    with password.open("rb") as stream:
                        call(
                            [
                                "gnome-keyring-daemon",
                                "--unlock",
                                "--control-directory",
                                str(runtime / "control"),
                            ],
                            stdin=stream,
                        )
                    unlocked = True
                try:
                    state = call(
                        [
                            "dbus-send",
                            "--session",
                            "--print-reply",
                            "--dest=org.freedesktop.secrets",
                            "/org/freedesktop/secrets/collection/login",
                            "org.freedesktop.DBus.Properties.Get",
                            "string:org.freedesktop.Secret.Collection",
                            "string:Locked",
                        ]
                    )
                    if "boolean false" in state:
                        return process
                except subprocess.CalledProcessError:
                    pass
            time.sleep(0.05)
        process.terminate()
        process.wait(timeout=10)
        raise RuntimeError(
            "Secret Service collection remains locked after unlock deadline"
        )

    def attempt(label: str, expected: str) -> None:
        begin = time.monotonic()
        result = call(["/usr/local/bin/credential_lifecycle", str(home / "storage")])
        results[label] = {
            "status": result,
            "seconds": round(time.monotonic() - begin, 3),
        }
        if result != expected:
            results[label]["credential_status"] = call(
                [
                    "/usr/local/bin/credential_lifecycle",
                    str(home / "storage"),
                    "diagnose",
                ]
            )
        results[label]["expected"] = expected
        results[label]["passed"] = result == expected

    def db_hash() -> str:
        return hashlib.sha256(
            (home / "storage/subscriptions.sqlite").read_bytes()
        ).hexdigest()

    try:
        for _ in range(100):
            if (runtime / "bus").exists():
                break
            time.sleep(0.05)
        assert "org.freedesktop.secrets" not in names()
        results["preflight"] = (
            "private bus; no activation service directories; no provider"
        )
        daemon = start()
        attempt("first_create", "ready")
        attempt("same_database_reopen", "ready")
        daemon.terminate()
        daemon.wait(timeout=10)
        daemon = None
        before = db_hash()
        attempt("unavailable", "keyring_locked")
        assert before == db_hash()
        results["unavailable"]["database_unchanged"] = True
        daemon = start()
        attempt("service_restart", "ready")
        call(
            [
                "dbus-send",
                "--session",
                "--print-reply",
                "--dest=org.freedesktop.secrets",
                "/org/freedesktop/secrets",
                "org.freedesktop.Secret.Service.Lock",
                "array:objpath:/org/freedesktop/secrets/collection/login",
            ]
        )
        before = db_hash()
        attempt("locked", "keyring_locked")
        assert before == db_hash()
        results["locked"]["database_unchanged"] = True
        print(json.dumps(results))
    finally:
        (home / "results.json").write_text(json.dumps(results))
        if daemon is not None:
            daemon.terminate()
            daemon.wait(timeout=10)
        bus.terminate()
        bus.wait(timeout=10)


if __name__ == "__main__":
    main()
