"""100 чередующихся циклов в одном DesktopHost; oracle родительского witness."""

from __future__ import annotations

import hashlib
import json
import os
import select
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from local_socks_resource import children, denied_probe, identity, oracle, sample

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from engine_compile import UUID
from engine_host import https_server, peer


def command(process: subprocess.Popen, text: str, expected: str) -> dict:
    if text:
        process.stdin.write(text + "\n")
        process.stdin.flush()
    if not select.select([process.stdout], [], [], 30)[0]:
        raise TimeoutError(expected)
    line = process.stdout.readline().strip()
    ack, _, raw = line.partition(" ")
    if ack != expected:
        raise RuntimeError("Нет ACK " + expected + ": " + line)
    return json.loads(raw) if raw else {}


def main() -> None:
    binary, engines, output = (Path(arg).resolve() for arg in sys.argv[1:4])
    output.mkdir(parents=True, exist_ok=True)
    peer_gate_only = sys.argv[4:] == ["--peer-gate-only"]
    if sys.argv[4:] and not peer_gate_only:
        raise ValueError("Неизвестный режим стенда")
    # Канарейка использует реальные собственные FD; production не получает fault.
    held = []
    canary = []
    try:
        for cycle in range(101):
            held.append(open("/dev/null", "rb"))
            canary.append(sample(os.getpid(), cycle))
        result = oracle(canary)
        assert result["result"] == "RED"
        (output / "canary.json").write_text(
            json.dumps(dict(summary=result, raw=canary), indent=2)
        )
    finally:
        for handle in held:
            handle.close()
    with tempfile.TemporaryDirectory(prefix="y-p05-") as temporary:
        root = Path(temporary)
        bus = subprocess.Popen(
            ["dbus-daemon", "--session", "--nofork", "--print-address=1"],
            stdout=subprocess.PIPE,
            text=True,
        )
        try:
            assert select.select([bus.stdout], [], [], 5)[0]
            address = bus.stdout.readline().strip()
            env = os.environ | {
                "HOME": str(root),
                "XDG_RUNTIME_DIR": str(root),
                "XDG_CONFIG_HOME": str(root / "config"),
                "XDG_DATA_HOME": str(root / "data"),
                "XDG_CACHE_HOME": str(root / "cache"),
                "DBUS_SESSION_BUS_ADDRESS": address,
                "DBUS_SYSTEM_BUS_ADDRESS": address,
            }
            with https_server(root, "health", 0, "127.0.0.1") as https:
                env["SSL_CERT_FILE"] = str(root / "health.ca.pem")
                for engine, directory in (("sing-box", "sing-box-1.14.4"),):
                    with socket.socket() as reserve:
                        reserve.bind(("127.0.0.1", 0))
                        peer_port = reserve.getsockname()[1]
                    with socket.socket() as reserve:
                        reserve.bind(("127.0.0.1", 0))
                        port = reserve.getsockname()[1]
                    core = engines / directory / engine
                    pins = {
                        "sing-box": (
                            "sing-box-1.14.4",
                            "1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a",
                        ),
                        "xray": (
                            "xray-26.10.31",
                            "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1",
                        ),
                    }
                    for name, (folder, digest) in pins.items():
                        assert (
                            hashlib.sha256(
                                (engines / folder / name).read_bytes()
                            ).hexdigest()
                            == digest
                        )
                    with peer(
                        core,
                        engine,
                        root,
                        port=peer_port,
                        target_ports=str(https.server_port),
                    ) as peer_process:
                        for denied_engine in pins:
                            denied_probe(
                                binary,
                                engines,
                                denied_engine,
                                root,
                                env,
                                peer_process,
                                peer_port,
                                port,
                                https,
                                output,
                            )
                        data = root / engine
                        data.mkdir(mode=0o700)
                        rows = []
                        tracked = []
                        connected = []
                        with (output / "alternating.stderr").open("w") as errors:
                            process = subprocess.Popen(
                                [
                                    str(binary),
                                    str(data),
                                    str(engines),
                                    engine,
                                    f"vless://{UUID}@127.0.0.1:{peer_port}#Synthetic",
                                    f"https://127.0.0.1:{https.server_port}/",
                                    str(port),
                                ],
                                env=env,
                                stdin=subprocess.PIPE,
                                stdout=subprocess.PIPE,
                                stderr=errors,
                                text=True,
                            )
                            try:
                                command(process, "", "ready")
                                rows.append(sample(process.pid, 0))
                                states = []
                                selected = None
                                for cycle in range(1, 3 if peer_gate_only else 101):
                                    https.requested.clear()
                                    state = command(process, "connect", "connect")
                                    current = "sing-box" if cycle % 2 else "xray"
                                    assert state["engine"] == current
                                    assert state["error_generation"] == cycle
                                    assert (
                                        state["preparing"]["error_generation"] == cycle
                                    )
                                    assert state["preparing"]["engine"] == current
                                    assert state["preparing"]["probe_result"] is None
                                    assert state["probe_result"] is not None
                                    if selected is None:
                                        selected = state["selected_profile"]
                                    assert state["selected_profile"] == selected
                                    states.append(dict(cycle=cycle, connected=state))
                                    assert https.requested.is_set(), (
                                        "Нет известного HTTPS через pinned peer"
                                    )
                                    owned = children(process.pid)
                                    assert len(owned) == 1
                                    pid = owned.pop()
                                    start = identity(pid)
                                    assert (
                                        hashlib.sha256(
                                            Path(f"/proc/{pid}/exe").read_bytes()
                                        ).hexdigest()
                                        == pins[current][1]
                                    )
                                    refusal = command(process, "refuse", "refuse")
                                    assert refusal["engine"] == current
                                    assert (
                                        children(process.pid) == {pid}
                                        and identity(pid) == start
                                    )
                                    assert refusal["selected_profile"] == selected
                                    states[-1]["refusal"] = refusal
                                    connected.append(
                                        dict(
                                            cycle=cycle,
                                            host=sample(process.pid, cycle),
                                            core=sample(pid, cycle),
                                        )
                                    )
                                    (output / "alternating.connected.json").write_text(
                                        json.dumps(connected, indent=2)
                                    )
                                    pidfd = os.pidfd_open(pid)
                                    try:
                                        states[-1]["stopped"] = command(
                                            process, "stop", "stop"
                                        )
                                        assert (
                                            states[-1]["stopped"]["error_generation"]
                                            == cycle
                                        )
                                        assert select.select([pidfd], [], [], 5)[0], (
                                            "Core жив после Stop"
                                        )
                                        assert (
                                            not Path(f"/proc/{pid}").exists()
                                            or identity(pid) != start
                                        ), "Core не reaped"
                                    finally:
                                        os.close(pidfd)
                                    with socket.socket() as rebound:
                                        rebound.setsockopt(
                                            socket.SOL_SOCKET, socket.SO_REUSEADDR, 1
                                        )
                                        rebound.bind(("127.0.0.1", port))
                                    if cycle < (2 if peer_gate_only else 100):
                                        states[-1]["switched"] = command(
                                            process, "switch", "switch"
                                        )
                                    (output / "states.json").write_text(
                                        json.dumps(states, indent=2)
                                    )
                                    time.sleep(0.05)
                                    rows.append(sample(process.pid, cycle))
                                    tracked.append(
                                        dict(
                                            cycle=cycle,
                                            engine=current,
                                            elf_sha256=pins[current][1],
                                            pid=pid,
                                            starttime=start,
                                            reaped=True,
                                            rebound=True,
                                            https=True,
                                        )
                                    )
                                    (output / "alternating.raw.json").write_text(
                                        json.dumps(rows, indent=2)
                                    )
                                pages = []
                                cursor = "0"
                                while True:
                                    process.stdin.write("logs:" + cursor + "\n")
                                    process.stdin.flush()
                                    assert select.select([process.stdout], [], [], 20)[
                                        0
                                    ]
                                    logs = json.loads(process.stdout.readline())
                                    assert len(logs["events"]) <= 256
                                    pages.append(logs)
                                    assert (
                                        sum(len(page["events"]) for page in pages)
                                        <= 2000
                                    )
                                    if not logs["events"]:
                                        break
                                    assert int(logs["cursor"]) > int(cursor)
                                    cursor = logs["cursor"]
                                (output / "alternating.logs.json").write_text(
                                    json.dumps(pages, indent=2)
                                )
                                command(process, "shutdown", "closed")
                                assert process.wait(timeout=10) == 0
                            finally:
                                if process.poll() is None:
                                    # Только собственный Popen; EOF инициирует Guard shutdown.
                                    process.stdin.close()
                                    process.wait(timeout=25)
                                process.stdout.close()
                                if not process.stdin.closed:
                                    process.stdin.close()
                        summary = (
                            {
                                "result": "PEER_NEGATIVE_AND_RESTORED_ALTERNATING_POSITIVE"
                            }
                            if peer_gate_only
                            else oracle(rows)
                        )
                        (output / "alternating.json").write_text(
                            json.dumps(
                                dict(
                                    summary=summary,
                                    tracked=tracked,
                                    core_sha256=hashlib.sha256(
                                        core.read_bytes()
                                    ).hexdigest(),
                                    closed=True,
                                ),
                                indent=2,
                            )
                        )
                        assert summary["result"] != "RED", summary
        finally:
            bus.terminate()
            bus.wait(timeout=5)
            bus.stdout.close()
    (output / "binary.sha256").write_text(
        hashlib.sha256(binary.read_bytes()).hexdigest() + "\n"
    )


if __name__ == "__main__":
    main()
