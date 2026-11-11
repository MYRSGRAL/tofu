"""Один production case без повторов RPC/traffic; только собственная guarded VM."""

from __future__ import annotations

import ctypes as c
import fcntl
import hashlib
import json
import os
import pwd
import selectors
import socket
import stat
import subprocess
import sys
import threading
import time
from pathlib import Path

BASE = Path("/opt/yonik-adapter-case")
SERVICE = b"cloud.highpriority.yonik.Network1"
OBJECT = b"/cloud/highpriority/yonik/Network"


def guard() -> None:
    assert os.environ.get("YONIK_PRODUCTION_HELPER_VM") == "1"
    assert Path("/sys/class/dmi/id/sys_vendor").read_text().strip() == "QEMU"
    assert Path("/proc/1/comm").read_text().strip() == "systemd"
    marker = Path("/etc/yonik-production-helper-vm")
    assert marker.stat().st_uid == 0 and marker.stat().st_mode & 0o022 == 0
    assert marker.read_text() == "yonik-production-helper-disposable-v1\n"


def emit(value: dict) -> None:
    print(json.dumps(value, ensure_ascii=False), flush=True)


class Bus:
    def __init__(self) -> None:
        self.lib = c.CDLL("libdbus-1.so.3")
        definitions = {
            "dbus_threads_init_default": (c.c_int, []),
            "dbus_bus_get_private": (c.c_void_p, [c.c_int, c.c_void_p]),
            "dbus_connection_set_exit_on_disconnect": (None, [c.c_void_p, c.c_int]),
            "dbus_message_new_method_call": (c.c_void_p, [c.c_char_p] * 4),
            "dbus_message_append_args": (c.c_int, [c.c_void_p, c.c_int]),
            "dbus_connection_send_with_reply": (
                c.c_int,
                [c.c_void_p, c.c_void_p, c.POINTER(c.c_void_p), c.c_int],
            ),
            "dbus_pending_call_block": (None, [c.c_void_p]),
            "dbus_pending_call_steal_reply": (c.c_void_p, [c.c_void_p]),
            "dbus_pending_call_unref": (None, [c.c_void_p]),
            "dbus_message_get_args": (c.c_int, [c.c_void_p, c.c_void_p]),
            "dbus_message_get_type": (c.c_int, [c.c_void_p]),
            "dbus_message_get_error_name": (c.c_char_p, [c.c_void_p]),
            "dbus_message_unref": (None, [c.c_void_p]),
            "dbus_connection_close": (None, [c.c_void_p]),
            "dbus_connection_unref": (None, [c.c_void_p]),
        }
        for name, (result, arguments) in definitions.items():
            fn = getattr(self.lib, name)
            fn.restype, fn.argtypes = result, arguments
        assert self.lib.dbus_threads_init_default()
        self.connection = self.lib.dbus_bus_get_private(1, None)
        assert self.connection
        self.lib.dbus_connection_set_exit_on_disconnect(self.connection, 0)

    def close(self) -> None:
        self.lib.dbus_connection_close(self.connection)
        self.lib.dbus_connection_unref(self.connection)

    def call(self, method: str, value: dict) -> dict:
        payload = json.dumps(value).encode()
        assert len(payload) <= 65536
        message = self.lib.dbus_message_new_method_call(
            SERVICE, OBJECT, SERVICE, method.encode()
        )
        assert message
        pending = c.c_void_p()
        reply = None
        try:
            text = c.c_char_p(payload)
            assert self.lib.dbus_message_append_args(
                message, ord("s"), c.byref(text), c.c_int(0)
            )
            assert self.lib.dbus_connection_send_with_reply(
                self.connection, message, c.byref(pending), 40000
            )
            assert pending.value
            self.lib.dbus_pending_call_block(pending)
            reply = self.lib.dbus_pending_call_steal_reply(pending)
            assert reply
            body = c.c_char_p()
            assert self.lib.dbus_message_get_args(
                reply, None, c.c_int(ord("s")), c.byref(body), c.c_int(0)
            )
            assert body.value is not None and len(body.value) <= 65536
            result = json.loads(body.value)
            if self.lib.dbus_message_get_type(reply) == 3:
                result = {
                    "error": result.get("error", "untyped"),
                    "error_name": self.lib.dbus_message_get_error_name(reply).decode(),
                }
            emit(
                {
                    "event": "rpc",
                    "method": method,
                    "state": result.get("state"),
                    "error": result.get("error"),
                }
            )
            return result
        finally:
            if reply:
                self.lib.dbus_message_unref(reply)
            if pending.value:
                self.lib.dbus_pending_call_unref(pending)
            self.lib.dbus_message_unref(message)


def request(engine: str, app: bool, kill_switch: bool = True) -> dict:
    options = {
        name: None
        for name in [
            "sni",
            "fp",
            "alpn",
            "pbk",
            "sid",
            "flow",
            "host",
            "path",
            "mode",
            "service_name",
            "authority",
            "spx",
            "cipher",
        ]
    }
    server = {
        "protocol": "vless",
        "host": "10.203.0.2",
        "port": 24443,
        "credentials": {
            "kind": "uuid",
            "uuid": "00112233-4455-6677-8899-aabbccddeeff",
            "username": None,
            "password": None,
            "method": None,
        },
        "transport": "tcp",
        "security": "none",
        "options": options,
    }
    rules = (
        [
            {
                "id": 1,
                "enabled": True,
                "action": "block",
                "domain": [],
                "domain_suffix": [],
                "ip_cidr": [],
                "executable_paths": ["/usr/bin/python3"],
            }
        ]
        if app
        else []
    )
    return {
        "version": 1,
        "mode": "system_profile_policy",
        "engine": engine,
        "kill_switch": kill_switch,
        "server": server,
        "policy": {
            "version": 1,
            "default_action": "proxy",
            "user_rules": rules,
            "preset_rules": [],
            "lan": [],
            "private_networks": [],
            "private_domains": [],
        },
    }


def client(
    engine: str, app: bool, kill_switch: bool = True, ending: str = "stop"
) -> None:
    assert os.getuid() == pwd.getpwnam("yonik-vm-allowed").pw_uid
    bus = Bus()
    stage = "initial_state"
    try:
        initial = bus.call("State", {"version": 1})
        assert initial.get("state") == "no_managed_policy", (
            "initial State != no_managed_policy"
        )
        stage = "prepare"
        prepared = bus.call("Prepare", request(engine, app, kill_switch))
        if app:
            assert prepared.get("error") == "unsupported", "Expected typed Unsupported"
        else:
            assert (
                isinstance(prepared.get("handle"), list)
                and len(prepared["handle"]) == 32
            )
            stage = "apply"
            assert (
                bus.call("Apply", {"version": 1, "handle": prepared["handle"]}).get(
                    "state"
                )
                == "tunnel_starting"
            )
            emit({"event": "inspect_worker"})
            assert sys.stdin.readline().strip() == "READY", (
                "Production worker inspection failed"
            )
            stage = "traffic"
            for family, address in [
                (socket.AF_INET, "198.18.0.2"),
                (socket.AF_INET6, "2001:db8:746f::2"),
            ]:
                route = json.loads(
                    subprocess.check_output(
                        ["ip", "-j", "route", "get", address, "uid", str(os.getuid())]
                    )
                )[0]
                assert route.get("dev") not in {
                    None,
                    "lo",
                    "eth0",
                    "yonik-outer",
                    "foreign0",
                }, "Traffic bypasses TUN"
                for kind in [socket.SOCK_STREAM, socket.SOCK_DGRAM]:
                    with socket.socket(family, kind) as connection:
                        connection.settimeout(4)
                        connection.connect((address, 24081))
                        payload = b"p" * 32
                        connection.sendall(payload)
                        expected = b"yonik-tun:" + payload
                        received = connection.recv(len(expected))
                        if kind == socket.SOCK_STREAM:
                            while len(received) < len(expected):
                                part = connection.recv(len(expected) - len(received))
                                assert part
                                received += part
                        assert received == expected
                    emit(
                        {
                            "event": "traffic",
                            "family": int(family),
                            "kind": int(kind),
                            "result": "PASS",
                        }
                    )
            if ending != "stop":
                stage = "ending_" + ending
                emit({"event": "ending", "ending": ending})
                assert sys.stdin.readline().strip() == "READY", (
                    "Crash/recovery inspection failed"
                )
                # Только read-only State может быть Busy во время safety sweep.
                # Unknown/неожиданный state не скрываются повторением.
                expected = "recovery_required" if kill_switch else "no_managed_policy"
                for attempt in range(20):
                    observed = bus.call("State", {"version": 1})
                    if observed.get("error") != "busy":
                        assert observed.get("state") == expected, (
                            "Recovery State mismatch"
                        )
                        break
                    assert attempt < 19, "Recovery State Busy deadline"
                    time.sleep(0.25)
            else:
                stage = "stop"
                assert bus.call("Stop", {"version": 1}).get("state") == (
                    "recovery_required" if kill_switch else "no_managed_policy"
                )
            stage = "kill_switch_witness"
            try:
                with socket.create_connection(("10.203.0.2", 24443), timeout=0.7):
                    reachable = True
            except OSError:
                reachable = False
            assert reachable == (not kill_switch), (
                "Direct reachability violates kill-switch disposition"
            )
            emit({"event": "endpoint_after_ending", "reachable": reachable})
            stage = "unlock"
            assert (
                bus.call("Unlock", {"version": 1}).get("state") == "no_managed_policy"
            )
            stage = "endpoint_restored"
            with socket.create_connection(("10.203.0.2", 24443), timeout=2):
                emit({"event": "endpoint_after_unlock", "reachable": True})
        stage = "final_state"
        assert bus.call("State", {"version": 1}).get("state") == "no_managed_policy"
        emit({"event": "result", "result": "PASS", "engine": engine, "app": app})
    except Exception as error:
        emit(
            {
                "event": "result",
                "result": "FAIL",
                "stage": stage,
                "kind": type(error).__name__,
                "reason": str(error)[:160],
            }
        )
        raise
    finally:
        bus.close()


def groups() -> dict:
    result = {}
    for name in ["yonik-worker", "yonik-worker-check"]:
        path = Path("/sys/fs/cgroup/system.slice") / (name + ".service/cgroup.events")
        try:
            result[name] = dict(
                line.split() for line in path.read_text().splitlines()
            ).get("populated", "unknown")
        except FileNotFoundError:
            result[name] = "absent"
    return result


class Sampler:
    def __init__(self, path: Path) -> None:
        self.path, self.stop = path, threading.Event()
        self.thread = threading.Thread(target=self.run)

    def run(self) -> None:
        previous = None
        with self.path.open("w") as log:
            while not self.stop.is_set():
                before = groups()
                try:
                    journal = json.loads(
                        Path("/run/yonik/network/journal.json").read_text()
                    )
                    value = {
                        "records": len(journal["records"]),
                        "states": [
                            record.get("state") for record in journal["records"]
                        ],
                    }
                except (OSError, KeyError, json.JSONDecodeError):
                    value = {"records": None}
                after = groups()
                value.update(before=before, after=after)
                if value != previous:
                    log.write(
                        json.dumps({"time_ns": time.monotonic_ns(), **value}) + "\n"
                    )
                    log.flush()
                    previous = value
                self.stop.wait(0.005)


def inspect_worker(engine: str) -> dict:
    expected = json.loads((BASE / "manifest-before.json").read_text())["engines"][
        engine
    ]
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        pid = int(
            subprocess.check_output(
                [
                    "systemctl",
                    "show",
                    "--property=MainPID",
                    "--value",
                    "yonik-worker.service",
                ]
            )
        )
        if pid > 1:
            try:
                path = Path(f"/proc/{pid}")
                with (path / "exe").open("rb") as binary:
                    digest = hashlib.file_digest(binary, "sha256").hexdigest()
                if digest == expected:
                    status = (path / "status").read_text()
                    assert "CapEff:\t0000000000001000" in status
                    assert "NoNewPrivs:\t1" in status
                    assert f"Uid:\t{pwd.getpwnam('yonik-worker').pw_uid}\t" in status
                    with (path / "fd/0").open("rb") as config:
                        metadata = os.fstat(config.fileno())
                        seals = fcntl.fcntl(config, fcntl.F_GET_SEALS)
                        assert stat.S_ISREG(metadata.st_mode) and seals & 15 == 15
                        assert stat.S_IMODE(metadata.st_mode) == 0o600
                        if engine == "sing-box":
                            assert (
                                metadata.st_uid == pwd.getpwnam("yonik-worker").pw_uid
                                and stat.S_IMODE(metadata.st_mode) == 0o600
                            )
                        config_hash = hashlib.file_digest(config, "sha256").hexdigest()
                    return {
                        "pid": pid,
                        "native_sha256": digest,
                        "status": status,
                        "stdin_uid": metadata.st_uid,
                        "stdin_mode": oct(stat.S_IMODE(metadata.st_mode)),
                        "stdin_seals": seals,
                        "config_sha256": config_hash,
                        "cgroup": (path / "cgroup").read_text(),
                        "mountinfo": (path / "mountinfo").read_text(),
                    }
            except FileNotFoundError:
                pass
        time.sleep(0.02)
    raise RuntimeError("Native worker readiness/identity deadline")


def helper_identity() -> dict:
    values = subprocess.check_output(
        [
            "systemctl",
            "show",
            "yonik-helper.service",
            "-p",
            "MainPID",
            "-p",
            "ActiveState",
            "-p",
            "NRestarts",
        ],
        text=True,
    )
    data = dict(line.split("=", 1) for line in values.splitlines())
    pid = int(data["MainPID"])
    assert pid > 1 and data["ActiveState"] == "active"
    with Path(f"/proc/{pid}/exe").open("rb") as binary:
        digest = hashlib.file_digest(binary, "sha256").hexdigest()
    expected = json.loads((BASE / "manifest-before.json").read_text())["files"][
        "yonik-helper"
    ]
    assert digest == expected
    return {"pid": pid, "exe_sha256": digest, "restarts": int(data["NRestarts"])}


def end_and_observe(ending: str, kill_switch: bool) -> dict:
    before = helper_identity()
    assert ending in {"restart", "worker_exit"}
    unit = "yonik-helper.service" if ending == "restart" else "yonik-worker.service"
    subprocess.run(
        ["systemctl", "kill", "--kill-whom=main", "--signal=SIGKILL", unit], check=True
    )
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        try:
            after = helper_identity()
            valid = (
                after["pid"] != before["pid"]
                if ending == "restart"
                else after["pid"] == before["pid"]
            )
            journal = json.loads(Path("/run/yonik/network/journal.json").read_text())
            empty = all(value in {"0", "absent"} for value in groups().values())
            retained = journal["records"]
            expected_records = (
                len(retained) == 1
                and retained[0].get("scope") == "PublicBlock"
                and retained[0].get("state") == "Confirmed"
                and journal.get("public_policy") is not None
                if kill_switch
                else not retained and journal.get("public_policy") is None
            )
            if (
                valid
                and empty
                and expected_records
                and journal.get("session_recovery") is None
            ):
                if ending == "restart":
                    assert after["restarts"] > before["restarts"]
                else:
                    assert after["restarts"] == before["restarts"]
                return {
                    "ending": ending,
                    "signal_unit": unit,
                    "helper_before": before,
                    "helper_after": after,
                    "automatic_worker_and_network_cleanup": True,
                    "retained_public_block": kill_switch,
                }
        except (AssertionError, FileNotFoundError, KeyError, json.JSONDecodeError):
            pass
        time.sleep(0.1)
    raise TimeoutError("Automatic recovery deadline; no Stop/Unlock recovery retry")


def coordinator(
    engine: str,
    app: bool,
    kill_switch: bool = True,
    ending: str = "stop",
    matrix: bool = False,
) -> None:
    assert os.geteuid() == 0
    label = engine + ("-app-denial" if app else "-positive")
    if matrix:
        label = f"{engine}-{str(kill_switch).lower()}-{ending}"
    output = BASE / "results"
    output.mkdir(exist_ok=True)
    sampler = Sampler(output / (label + "-transitions.jsonl"))
    sampler.thread.start()
    child = subprocess.Popen(
        [
            "runuser",
            "-u",
            "yonik-vm-allowed",
            "--",
            sys.executable,
            str(Path(__file__).resolve()),
            "client",
            engine,
            "app" if app else "positive",
            "true" if kill_switch else "false",
            ending,
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=False,
        bufsize=0,
        start_new_session=True,
    )
    events = []
    buffered = b""
    try:
        selector = selectors.DefaultSelector()
        selector.register(child.stdout, selectors.EVENT_READ)
        deadline = time.monotonic() + 90
        with (output / (label + "-events.jsonl")).open("w") as log:
            while time.monotonic() < deadline:
                if not selector.select(timeout=1):
                    if child.poll() is not None:
                        break
                    continue
                chunk = os.read(child.stdout.fileno(), 65536)
                if not chunk:
                    break
                buffered += chunk
                while b"\n" in buffered:
                    line, buffered = buffered.split(b"\n", 1)
                    line = line.decode()
                    event = json.loads(line)
                    events.append(event)
                    log.write(line + "\n")
                    log.flush()
                    if event.get("event") == "ending":
                        data = end_and_observe(ending, kill_switch)
                        (output / (label + "-ending.json")).write_text(
                            json.dumps(data, indent=2)
                        )
                        child.stdin.write(b"READY\n")
                        child.stdin.flush()
                    if event.get("event") == "inspect_worker":
                        data = inspect_worker(engine)
                        (output / (label + "-worker.json")).write_text(
                            json.dumps(data, indent=2)
                        )
                        child.stdin.write(b"READY\n")
                        child.stdin.flush()
            else:
                raise TimeoutError("Case deadline")
        child.wait(timeout=5)
        (output / (label + "-stderr.log")).write_text(child.stderr.read().decode())
        result = {
            "engine": engine,
            "app": app,
            "exit": child.returncode,
            "result": events[-1]
            if events
            else {"result": "FAIL", "stage": "no_events"},
        }
        (output / (label + "-result.json")).write_text(json.dumps(result, indent=2))
        emit(result)
        assert child.returncode == 0 and events[-1].get("result") == "PASS"
    except Exception as error:
        path = output / (label + "-result.json")
        if not path.exists():
            path.write_text(
                json.dumps(
                    {
                        "result": "FAIL",
                        "stage": "controller_or_inspect",
                        "kind": type(error).__name__,
                        "reason": str(error)[:160],
                    },
                    indent=2,
                )
            )
        raise
    finally:
        if child.poll() is None:
            import signal

            os.killpg(child.pid, signal.SIGKILL)
            child.wait(timeout=5)
        sampler.stop.set()
        sampler.thread.join(timeout=2)
    if app:
        return
    samples = [json.loads(line) for line in sampler.path.read_text().splitlines()]
    live = next(
        index for index, sample in enumerate(samples) if "1" in sample["after"].values()
    )
    drops = []
    for previous, current in zip(samples[live:], samples[live + 1 :]):
        if (
            previous["records"] is not None
            and current["records"] is not None
            and current["records"] < previous["records"]
        ):
            assert all(
                value in {"0", "absent"}
                for side in ["before", "after"]
                for value in current[side].values()
            ), "Network cleanup observed before empty worker cgroups"
            drops.append(current)
    assert drops and samples[-1]["records"] == 0
    (output / (label + "-cleanup.json")).write_text(
        json.dumps({"worker_first_drops": drops, "final_cgroups": groups()}, indent=2)
    )


if __name__ == "__main__":
    guard()
    if sys.argv[1] == "client":
        client(
            sys.argv[2],
            sys.argv[3] == "app",
            len(sys.argv) < 5 or sys.argv[4] == "true",
            sys.argv[5] if len(sys.argv) > 5 else "stop",
        )
    else:
        coordinator(
            sys.argv[2],
            sys.argv[3] == "app",
            len(sys.argv) < 5 or sys.argv[4] == "true",
            sys.argv[5] if len(sys.argv) > 5 else "stop",
            sys.argv[1] == "matrix",
        )
