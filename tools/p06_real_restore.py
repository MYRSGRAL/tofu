"""Один изолированный real Secret Service maintenance witness без чтения ключа."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import selectors
import socket
import subprocess
import tempfile
import time
from pathlib import Path


def run(binary: Path, output: Path) -> None:
    """Проверить нормальный цикл и закрыть только собственные процессы."""
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    processes: list[subprocess.Popen] = []
    with tempfile.TemporaryDirectory(prefix="p06-real-") as temporary:
        root = Path(temporary)
        nonce = os.urandom(16).hex()
        (root / "nonce").write_text(nonce)
        for directory in ("copies", "restored", "negative"):
            (root / directory).mkdir(mode=0o700)
        env = dict(
            PATH=os.defpath,
            LANG="C.UTF-8",
            HOME=temporary,
            XDG_RUNTIME_DIR=temporary,
            XDG_DATA_HOME=temporary + "/data",
            XDG_CONFIG_HOME=temporary + "/config",
            XDG_CACHE_HOME=temporary + "/cache",
            QT_QPA_PLATFORM="offscreen",
            P06_PRIVATE_NONCE=nonce,
        )
        config = root / "bus.conf"
        config.write_text(
            "<busconfig><type>session</type><listen>unix:tmpdir="
            + temporary
            + '</listen><policy context="default"><allow send_destination="*"/>'
            '<allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
        )
        records: dict = {
            "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()
        }

        def call(*args: str) -> str:
            return subprocess.check_output(
                ["busctl", "--address=" + address, *args],
                env=env,
                stderr=subprocess.DEVNULL,
                timeout=5,
                text=True,
            ).strip()

        def inspect() -> dict:
            pid = call(
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetConnectionUnixProcessID",
                "s",
                "org.freedesktop.secrets",
            )
            uid = call(
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetConnectionUnixUser",
                "s",
                "org.freedesktop.secrets",
            )
            assert (
                pid == f"u {daemon.pid}"
                and uid == f"u {os.getuid()}"
                and daemon.poll() is None
            )
            collection = call(
                "call",
                "org.freedesktop.secrets",
                "/org/freedesktop/secrets",
                "org.freedesktop.Secret.Service",
                "ReadAlias",
                "s",
                "default",
            ).split('"')[1]
            items = call(
                "get-property",
                "org.freedesktop.secrets",
                collection,
                "org.freedesktop.Secret.Collection",
                "Items",
            )
            locked = call(
                "get-property",
                "org.freedesktop.secrets",
                collection,
                "org.freedesktop.Secret.Collection",
                "Locked",
            )
            assert locked == "b false"
            return {
                "owner_pid": daemon.pid,
                "owner_uid": os.getuid(),
                "collection": collection,
                "items": items,
                "locked": False,
                "attributes": [
                    call(
                        "get-property",
                        "org.freedesktop.secrets",
                        item,
                        "org.freedesktop.Secret.Item",
                        "Attributes",
                    )
                    for item in items.split(chr(34))[1::2]
                ],
            }

        try:
            bus = subprocess.Popen(
                [
                    "dbus-daemon",
                    "--nofork",
                    "--print-address=1",
                    "--config-file=" + str(config),
                ],
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
            )
            processes.append(bus)
            with selectors.DefaultSelector() as selector:
                selector.register(bus.stdout, selectors.EVENT_READ)
                assert selector.select(5)
            address = bus.stdout.readline().decode().strip()
            env.update(
                DBUS_SESSION_BUS_ADDRESS=address, DBUS_SYSTEM_BUS_ADDRESS=address
            )
            assert (
                call(
                    "call",
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "NameHasOwner",
                    "s",
                    "org.freedesktop.secrets",
                )
                == "b false"
            )
            records["name_absent_before"] = True
            key_read, key_write = os.pipe()
            with (
                socket.socket(socket.AF_UNIX) as listener,
                (output / "daemon.log").open("wb") as log,
            ):
                listener.bind(temporary + "/environment")
                listener.listen(1)
                daemon = subprocess.Popen(
                    ["ksecretd", "--pam-login", str(key_read), str(listener.fileno())],
                    env=env | {"PAM_KWALLET5_LOGIN": "1"},
                    pass_fds=(key_read, listener.fileno()),
                    stdout=log,
                    stderr=log,
                )
                processes.append(daemon)
                os.close(key_read)
                os.write(key_write, os.urandom(56))
                os.close(key_write)
                with socket.socket(socket.AF_UNIX) as connection:
                    connection.connect(temporary + "/environment")
                deadline = time.monotonic() + 5
                while True:
                    try:
                        records["before"] = inspect()
                        break
                    except subprocess.CalledProcessError:
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(0.1)
                assert records["before"]["items"] == "ao 0"
                for stage in (
                    "seed",
                    "backup",
                    "restore",
                    "negative",
                    "reopen",
                    "verify",
                ):
                    result = subprocess.run(
                        [str(binary), temporary, stage],
                        env=env,
                        capture_output=True,
                        timeout=90,
                    )
                    (output / (stage + ".log")).write_bytes(
                        result.stdout + result.stderr
                    )
                    assert result.returncode == 0, stage
                    records[stage] = inspect()
                    source_hash = hashlib.sha256(
                        (root / "live/subscriptions.sqlite").read_bytes()
                    ).hexdigest()
                    if stage == "seed":
                        records["source_sha256"] = source_hash
                    else:
                        assert source_hash == records["source_sha256"]
                    assert records[stage]["items"].startswith("ao 1 ")
                    if stage != "seed":
                        assert records[stage] == records["seed"]
                records["stages_passed"] = True
        finally:
            try:
                if len(processes) == 2 and processes[0].poll() is None:
                    daemon.terminate()
                    daemon.wait(timeout=5)
                    records["name_absent_after"] = (
                        call(
                            "call",
                            "org.freedesktop.DBus",
                            "/org/freedesktop/DBus",
                            "org.freedesktop.DBus",
                            "NameHasOwner",
                            "s",
                            "org.freedesktop.secrets",
                        )
                        == "b false"
                    )
            finally:
                for process in reversed(processes):
                    if process.poll() is None:
                        process.terminate()
                        process.wait(timeout=5)
                (output / "result.json").write_text(
                    json.dumps(records, indent=2) + "\n"
                )
        assert records.get("stages_passed") and records.get("name_absent_after")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    run(args.binary.resolve(), args.output.resolve())
