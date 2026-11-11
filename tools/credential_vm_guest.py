"""Guest-only controller: отдельный пользователь, bus без autolaunch, свой keyring."""

import hashlib
import http.server
import json
import os
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import threading
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
        "database_existed_before_run": (home / "storage/subscriptions.sqlite").exists(),
        "packages": subprocess.run(
            ["pacman", "-Q", "gnome-keyring", "dbus"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.splitlines(),
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
        for _ in range(200):
            if "org.freedesktop.secrets" in names():
                break
            time.sleep(0.05)
        else:
            process.terminate()
            process.wait(timeout=10)
            raise RuntimeError("Нет Secret Service owner")
        results.setdefault("daemon_starts", []).append(
            {
                "pid": process.pid,
                "control_directory": str(runtime / "control"),
                "after_startup_unlock": inspect(),
            }
        )
        state = inspect()
        results["daemon_starts"][-1]["startup_unlock"] = state
        assert state["owner_pid"] == process.pid
        return process

    def bus_call(
        path: str, interface: str, method: str, signature: str = "", *args: str
    ) -> object:
        command = [
            "busctl",
            "--address=" + env["DBUS_SESSION_BUS_ADDRESS"],
            "--json=short",
            "call",
            "org.freedesktop.secrets",
            path,
            interface,
            method,
        ]
        if signature:
            command.extend([signature, *args])
        return json.loads(call(command))["data"]

    def inspect() -> dict[str, object]:
        result = {}
        result["owner"] = json.loads(
            call(
                [
                    "busctl",
                    "--address=" + env["DBUS_SESSION_BUS_ADDRESS"],
                    "--json=short",
                    "call",
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "GetNameOwner",
                    "s",
                    "org.freedesktop.secrets",
                ]
            )
        )["data"]
        result["owner_pid"] = json.loads(
            call(
                [
                    "busctl",
                    "--address=" + env["DBUS_SESSION_BUS_ADDRESS"],
                    "--json=short",
                    "call",
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "GetConnectionUnixProcessID",
                    "s",
                    "org.freedesktop.secrets",
                ]
            )
        )["data"][0]
        for alias in ["default", "login"]:
            try:
                path = bus_call(
                    "/org/freedesktop/secrets",
                    "org.freedesktop.Secret.Service",
                    "ReadAlias",
                    "s",
                    alias,
                )[0]
                result[alias] = {"path": path}
                if path != "/":
                    state = json.loads(
                        call(
                            [
                                "busctl",
                                "--address=" + env["DBUS_SESSION_BUS_ADDRESS"],
                                "--json=short",
                                "get-property",
                                "org.freedesktop.secrets",
                                path,
                                "org.freedesktop.Secret.Collection",
                                "Locked",
                            ]
                        )
                    )["data"]
                    result[alias]["locked"] = state
            except subprocess.CalledProcessError:
                result[alias] = {"status": "unavailable"}
        return result

    def unlock_collection() -> None:
        # Протокол gkd-control: проверяем peer PID/UID, ответ ровно 8 байт.
        path = runtime / "control/control"
        metadata = path.lstat()
        assert stat.S_ISSOCK(metadata.st_mode) and metadata.st_uid == os.getuid()
        password = bytearray((home / "password").read_bytes())
        assert 0 < len(password) <= 128
        packet = bytearray(struct.pack("!III", 12 + len(password), 1, len(password)))
        packet.extend(password)
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as control:
                control.settimeout(3)
                control.connect(str(path))
                pid, uid, _ = struct.unpack(
                    "3i", control.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
                )
                assert pid == daemon.pid and uid == os.getuid()
                control.sendall(b"\x00")
                control.sendall(packet)
                response = control.recv(8, socket.MSG_WAITALL)
                assert len(response) == 8 and struct.unpack("!II", response) == (8, 0)
        finally:
            password[:] = bytes(len(password))
            packet[:] = bytes(len(packet))

    def attempt(label: str, expected: str) -> None:
        begin = time.monotonic()
        process = subprocess.Popen(
            ["/usr/local/bin/credential_lifecycle", str(home / "storage")],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        output, _ = process.communicate(timeout=30)
        result = output.decode().strip()
        assert result in ["ready", "keyring_locked", "keyring_missing", "other_failure"]

        results[label] = {
            "status": result,
            "pid": process.pid,
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
        try:
            results[label]["get_secret_state"] = json.loads(
                call(["/usr/local/bin/credential_probe"])
            )
        except subprocess.CalledProcessError:
            results[label]["get_secret_state"] = "service_unavailable"
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

        class Fixture(http.server.BaseHTTPRequestHandler):
            def do_GET(self) -> None:
                body = b"trojan://vm-fixture@example.invalid:443#fixture"
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, format: str, *args: object) -> None:
                pass

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            results["profile_seed"] = call(
                [
                    "/usr/local/bin/credential_lifecycle",
                    str(home / "storage"),
                    "seed",
                    f"http://127.0.0.1:{server.server_port}/fixture",
                ]
            )
        except subprocess.CalledProcessError:
            results["profile_seed"] = "failed"
        finally:
            server.shutdown()
            server.server_close()
        attempt("same_database_reopen", "ready")
        try:
            results["profile_reopen"] = call(
                [
                    "/usr/local/bin/credential_lifecycle",
                    str(home / "storage"),
                    "profile",
                ]
            )
        except subprocess.CalledProcessError:
            results["profile_reopen"] = "failed"

        daemon.terminate()
        daemon.wait(timeout=10)
        daemon = None
        assert "org.freedesktop.secrets" not in names()
        results["owner_absent_after_stop"] = True
        before = db_hash()
        attempt("unavailable", "keyring_locked")
        assert before == db_hash()
        results["unavailable"]["database_unchanged"] = True
        daemon = start()
        attempt("service_restart", "ready")
        try:
            results["profile_after_restart"] = call(
                [
                    "/usr/local/bin/credential_lifecycle",
                    str(home / "storage"),
                    "profile",
                ]
            )
        except subprocess.CalledProcessError:
            results["profile_after_restart"] = "failed"

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
        results["locked_collection_state"] = inspect()
        before_unlock = inspect()
        unlock_collection()
        after_unlock = inspect()
        assert before_unlock["owner"] == after_unlock["owner"]
        assert before_unlock["default"]["locked"] is True
        assert after_unlock["default"]["locked"] is False
        results["same_owner_collection_unlock"] = {
            "before": before_unlock,
            "after": after_unlock,
        }
        attempt("fresh_process_after_live_unlock", "ready")
        results["profile_after_live_unlock"] = call(
            ["/usr/local/bin/credential_lifecycle", str(home / "storage"), "profile"]
        )
        if len(sys.argv) > 1 and sys.argv[1] == "unlock":
            print(json.dumps(results))
            return

        daemon.terminate()
        daemon.wait(timeout=10)
        daemon = start()
        attempt("fresh_after_collection_unlock", "ready")
        try:
            results["profile_after_collection_unlock"] = call(
                [
                    "/usr/local/bin/credential_lifecycle",
                    str(home / "storage"),
                    "profile",
                ]
            )
        except subprocess.CalledProcessError:
            results["profile_after_collection_unlock"] = "failed"
        # Старый DH session/item удерживается через смену реального owner.
        stale = subprocess.Popen(
            ["/usr/local/bin/credential_probe", "stale"],
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        assert select.select([stale.stdout], [], [], 5)[0]
        results["stale_session_open"] = json.loads(stale.stdout.readline())
        old_owner = inspect()["owner"]
        daemon.terminate()
        daemon.wait(timeout=10)
        daemon = start()
        stale_output, _ = stale.communicate(input=b"x", timeout=20)
        results["stale_after_owner_change"] = json.loads(stale_output)
        results["owner_changed"] = old_owner != inspect()["owner"]
        attempt("fresh_after_owner_change", "ready")
        # Останавливаем daemon до запроса: production port находится в pending OpenSession.
        before = db_hash()
        os.kill(daemon.pid, signal.SIGSTOP)
        pending = subprocess.Popen(
            ["/usr/local/bin/credential_lifecycle", str(home / "storage")],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        time.sleep(0.2)
        assert pending.poll() is None
        begin = time.monotonic()
        daemon.terminate()
        os.kill(daemon.pid, signal.SIGCONT)
        daemon.wait(timeout=10)
        daemon = None
        output, _ = pending.communicate(timeout=22)
        results["pending_owner_loss"] = {
            "status": output.decode().strip(),
            "seconds": round(time.monotonic() - begin, 3),
            "database_unchanged": before == db_hash(),
            "pid": pending.pid,
        }
        assert results["pending_owner_loss"]["status"] == "keyring_locked"
        assert results["pending_owner_loss"]["database_unchanged"]
        # Новая private bus = новая сессия, те же persisted keyring и DB.
        bus.terminate()
        bus.wait(timeout=10)
        bus = subprocess.Popen(
            ["dbus-daemon", "--nofork", "--config-file", str(config)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        for _ in range(100):
            try:
                names()
                break
            except subprocess.CalledProcessError:
                time.sleep(0.05)
        assert "org.freedesktop.secrets" not in names()
        daemon = start()
        attempt("fresh_session_restart", "ready")
        results["profile_after_session_restart"] = call(
            ["/usr/local/bin/credential_lifecycle", str(home / "storage"), "profile"]
        )
        # Race использует отдельный пустой keyring; исходный профиль не перезаписывается.
        daemon.terminate()
        daemon.wait(timeout=10)
        env["XDG_DATA_HOME"] = str(home / "race-data")
        daemon = start()
        before = db_hash()
        attempt("existing_database_missing_key", "keyring_missing")
        assert before == db_hash()
        first = subprocess.Popen(
            ["/usr/local/bin/credential_probe", "create"],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        second = subprocess.Popen(
            ["/usr/local/bin/credential_probe", "create"],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        race = [
            json.loads(process.communicate(timeout=20)[0])
            for process in [first, second]
        ]
        count = json.loads(call(["/usr/local/bin/credential_probe"]))
        results["creation_race"] = {"results": race, "items": count}
        assert count["unlocked"] == 1 and count["locked"] == 0
        assert all(r["status"] in ["available", "Busy"] for r in race)
        assert any(r["status"] == "available" for r in race)
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
