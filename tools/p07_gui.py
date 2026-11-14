#!/usr/bin/env python3
"""P07: production qrc GUI, private loopback/TLS, batch cancel и terminal oracle."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import select
import shlex
import socket
import subprocess
import sys
import threading
import time
from contextlib import ExitStack
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from engine_host import https_server
from native_cancel_matrix import remaining, snapshot

ROOT = Path(__file__).resolve().parents[1]


def pending(port: int) -> int:
    return sum(
        line.split()[3] == "02" and int(line.split()[2].split(":")[1], 16) == port
        for line in Path("/proc/net/tcp").read_text().splitlines()[1:]
    )


def inside(args: argparse.Namespace) -> None:
    assert Path("/proc/1/ns/pid").readlink() == Path("/proc/self/ns/pid").readlink()
    assert os.readlink("/proc/self/ns/net") != args.parent_net
    assert [
        line.split(":")[0].strip()
        for line in Path("/proc/net/dev").read_text().splitlines()[2:]
    ] == ["lo"]
    args.output.mkdir(exist_ok=True)
    # Accepted interposer загружает оригинальный qrc Main, затем отдельный observer.
    spec = importlib.util.spec_from_file_location(
        "accepted", ROOT / "tools/tests/check_scheduler_production_ui.py"
    )
    assert spec and spec.loader
    accepted = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(ROOT / "tools/tests"))
    spec.loader.exec_module(accepted)
    cpp = args.output / "observer.cpp"
    cpp.write_text(accepted.INTERPOSE)
    library = args.output / "observer.so"
    flags = shlex.split(
        subprocess.check_output(
            ["pkg-config", "--cflags", "--libs", "Qt6Qml"], text=True
        )
    )
    subprocess.run(
        ["clang++", "-shared", "-fPIC", str(cpp), "-o", str(library), *flags, "-ldl"],
        check=True,
    )
    baseline = snapshot()
    results = []
    for engine in ("xray", "sing-box"):
        for repeat in range(3):
            work = args.output / f"{engine}-{repeat}"
            work.mkdir()
            with ExitStack() as stack:
                trusted = stack.enter_context(
                    https_server(work, "trusted", 0, "198.18.0.1")
                )
                untrusted = stack.enter_context(
                    https_server(work, "untrusted", 0, "198.18.0.1")
                )
                upstream = stack.enter_context(socket.socket())
                upstream.bind(("127.0.0.1", 0))
                upstream.listen()
                slow = stack.enter_context(socket.socket())
                slow.bind(("127.0.0.1", 0))
                slow.listen(0)
                filler = stack.enter_context(
                    socket.create_connection(slow.getsockname())
                )
                assert filler.fileno() >= 0
                peer_events: list[dict] = []
                peer_errors: list[str] = []
                done = threading.Event()
                workers: list[threading.Thread] = []

                def exact(stream: socket.socket, count: int) -> bytes:
                    data = b""
                    while len(data) < count:
                        chunk = stream.recv(count - len(data))
                        if not chunk:
                            raise EOFError("peer truncated")
                        data += chunk
                    return data

                def serve_peer(stream: socket.socket) -> None:
                    with stream:
                        stream.settimeout(12)
                        greeting = stream.recv(1)
                        if not greeting:  # check_batch делает connect без payload
                            return
                        greeting += exact(stream, 2)
                        assert greeting == b"\x05\x01\x00"
                        stream.sendall(b"\x05\x00")
                        request = exact(stream, 10)
                        assert request[:8] == bytes([5, 1, 0, 1, 198, 18, 0, 1])
                        port = int.from_bytes(request[8:], "big")
                        peer_events.append(
                            {
                                "port": port,
                                "request": request.hex(),
                                "time": time.monotonic(),
                            }
                        )
                        if port == 18082:
                            stream.sendall(bytes([5, 5, 0, 1, 0, 0, 0, 0, 0, 0]))
                            return
                        if port == 18083:
                            # Ответ SOCKS удержан: Stop обязан закрыть actual health socket.
                            assert stream.recv(1) == b""
                            peer_events.append(
                                {"held_eof": True, "time": time.monotonic()}
                            )
                            return
                        assert port in (18080, 18081)
                        tls_port = (
                            trusted.server_port
                            if port == 18080
                            else untrusted.server_port
                        )
                        with socket.create_connection(("127.0.0.1", tls_port)) as tls:
                            stream.sendall(bytes([5, 0, 0, 1, 127, 0, 0, 1, 0, 0]))
                            while not done.is_set():
                                ready, _, _ = select.select([stream, tls], [], [], 0.1)
                                for source in ready:
                                    chunk = source.recv(65536)
                                    if not chunk:
                                        return
                                    (tls if source is stream else stream).sendall(chunk)

                def checked_peer(stream: socket.socket) -> None:
                    try:
                        serve_peer(stream)
                    except (ConnectionResetError, BrokenPipeError):
                        peer_events.append({"transport_closed": True})
                    except Exception as error:
                        peer_errors.append(repr(error))

                def accept() -> None:
                    upstream.settimeout(0.1)
                    while not done.is_set():
                        try:
                            stream, _ = upstream.accept()
                        except TimeoutError:
                            continue
                        worker = threading.Thread(target=checked_peer, args=(stream,))
                        workers.append(worker)
                        worker.start()

                accept_thread = threading.Thread(target=accept)
                accept_thread.start()
                body = (
                    f"socks5://127.0.0.1:{upstream.getsockname()[1]}#A-health\n"
                    + "\n".join(
                        f"trojan://p07-{i}@127.0.0.1:{slow.getsockname()[1]}#Z-{i:03}"
                        for i in range(64)
                    )
                ).encode()

                class Handler(BaseHTTPRequestHandler):
                    def do_GET(self) -> None:
                        self.send_response(200)
                        self.send_header("Content-Length", str(len(body)))
                        self.end_headers()
                        self.wfile.write(body)

                    def log_message(self, _format: str, *values: object) -> None:
                        pass

                http = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
                http_thread = threading.Thread(target=http.serve_forever)
                http_thread.start()
                config = {
                    "engine": engine,
                    "source": f"http://127.0.0.1:{http.server_port}/",
                }
                driver = work / "driver.qml"
                driver.write_text(
                    (ROOT / "tools/p07_gui_driver.qml")
                    .read_text()
                    .replace("@CONFIG@", json.dumps(config))
                )
                env = dict(
                    os.environ,
                    LD_PRELOAD=str(library),
                    YONIK_UI_DRIVER=str(driver),
                    QT_QPA_PLATFORM="offscreen",
                    QT_QUICK_BACKEND="software",
                    QT_QUICK_CONTROLS_STYLE="Basic",
                    QT_FORCE_STDERR_LOGGING="1",
                    SSL_CERT_FILE=str(work / "trusted.ca.pem"),
                    SSL_CERT_DIR=str(work / "empty-ca"),
                )
                (work / "empty-ca").mkdir()
                samples = []
                markers = []
                start = time.monotonic()
                try:
                    with (work / "qt.log").open("w") as log:
                        process = subprocess.Popen(
                            [args.binary, str(work / "host"), args.cores],
                            env=env,
                            stdout=log,
                            stderr=log,
                        )
                        cancel_socket_proof = False
                        stop_runtime_proof = False
                        while process.poll() is None:
                            samples.append(
                                {
                                    "t": time.monotonic() - start,
                                    "resources": remaining(snapshot(), baseline),
                                    "pending_batch_sockets": pending(
                                        slow.getsockname()[1]
                                    ),
                                }
                            )
                            current_log = (work / "qt.log").read_text()
                            engine_pids = []
                            executables = {}
                            for key in samples[-1]["resources"]:
                                try:
                                    exe = os.readlink(f"/proc/{key.split(':')[0]}/exe")
                                except (FileNotFoundError, ProcessLookupError):
                                    continue
                                executables[key] = exe
                                if exe.startswith("/memfd:yonik-engine "):
                                    engine_pids.append(key)
                            samples[-1]["engine_pids"] = engine_pids
                            samples[-1]["executables"] = executables
                            with (work / "samples.jsonl").open("a") as samples_log:
                                samples_log.write(json.dumps(samples[-1]) + "\n")
                            if "P07 STOP_ACK" in current_log:
                                assert not engine_pids, "Stop retained engine runtime"
                                stop_runtime_proof = True
                            if "P07 BATCH_DRAINED" in current_log:
                                assert pending(slow.getsockname()[1]) == 0, (
                                    "CancelBatch retained SYN sockets"
                                )
                                if (
                                    not cancel_socket_proof
                                    and "P07 STOP_ACK" not in current_log
                                ):
                                    assert len(engine_pids) == 1, (
                                        f"CancelBatch runtime classifier: {executables}, engines={engine_pids}"
                                    )
                                    cancel_socket_proof = True
                            if time.monotonic() - start > 45:
                                process.kill()
                                process.wait()
                                raise RuntimeError("GUI deadline")
                            time.sleep(0.02)
                    text = (work / "qt.log").read_text()
                    for line in text.splitlines():
                        for marker in (
                            "P07 STRICT ",
                            "P07 HIDDEN ",
                            "P07 LOGS ",
                            "P07 GUI_PASS ",
                        ):
                            if marker in line:
                                markers.append(
                                    {
                                        "marker": marker.strip(),
                                        "value": json.loads(line.split(marker, 1)[1]),
                                    }
                                )
                    assert process.returncode == 0, text[-5000:]
                    assert (
                        "P07 FAIL" not in text
                        and "TypeError" not in text
                        and "ReferenceError" not in text
                    ), text[-5000:]
                    main_hash = hashlib.sha256(
                        (ROOT / "crates/yonik-desktop/qml/Main.qml").read_bytes()
                    ).hexdigest()
                    assert f"PRODUCTION MAIN SHA256 {main_hash}" in text
                    assert len([m for m in markers if m["marker"] == "P07 STRICT"]) == 3
                    assert all(
                        any(m["marker"] == name for m in markers)
                        for name in ("P07 HIDDEN", "P07 LOGS", "P07 GUI_PASS")
                    )
                    assert (
                        trusted.requested.is_set() and not untrusted.requested.is_set()
                    )
                    assert any(e.get("held_eof") for e in peer_events)
                    assert max(s["pending_batch_sockets"] for s in samples) > 0
                    assert pending(slow.getsockname()[1]) == 0
                    disk_files = {
                        p.name: p.stat().st_size
                        for p in (work / "host/data/logs").glob("events*.log")
                    }
                    assert disk_files and len(disk_files) <= 5
                    assert all(size <= 4 * 1024 * 1024 for size in disk_files.values())
                    assert cancel_socket_proof and stop_runtime_proof
                    assert not peer_errors, peer_errors
                    terminal = remaining(snapshot(), baseline)
                    assert not terminal, terminal
                    results.append(
                        {
                            "engine": engine,
                            "repeat": repeat,
                            "duration_s": time.monotonic() - start,
                            "markers": markers,
                            "peer_events": peer_events,
                            "samples": samples,
                            "terminal": terminal,
                            "disk_files": disk_files,
                            "strict_ca": True,
                            "peer_negative": True,
                        }
                    )
                    (args.output / "results.json").write_text(
                        json.dumps(results, indent=2)
                    )
                finally:
                    done.set()
                    accept_thread.join(2)
                    for worker in workers:
                        worker.join(2)
                    http.shutdown()
                    http.server_close()
                    http_thread.join(2)
    retainer = subprocess.Popen(
        [
            sys.executable,
            "-c",
            'import time; b=bytearray(8*1024*1024); f=open("/dev/null"); time.sleep(10)',
        ]
    )
    try:
        time.sleep(0.1)
        rows = remaining(snapshot(), baseline)
        assert rows and any(
            (r["private_dirty"] or 0) >= 8 * 1024 * 1024 and (r["fd"] or 0) > 0
            for r in rows.values()
        )
        (args.output / "negative.json").write_text(json.dumps(rows, indent=2))
    finally:
        retainer.terminate()
        retainer.wait()
    assert not remaining(snapshot(), baseline)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("cores")
    parser.add_argument("output", type=Path)
    parser.add_argument("--inside", action="store_true")
    parser.add_argument("--parent-net")
    args = parser.parse_args()
    if args.inside:
        inside(args)
        return
    private = (
        ROOT / "crates/yonik-desktop/examples/p07_gui/bridge_impl.rs"
    ).read_text()
    begin = private.index('        if request.to_string() == r#"{')
    end = private.index("        let Some(host) = self.host.as_ref() else {", begin)
    restored = (private[:begin] + private[end:]).replace(
        "../../src/bridge_pending.rs", "bridge_pending.rs"
    )
    assert restored == (ROOT / "crates/yonik-desktop/src/bridge_impl.rs").read_text(), (
        "Неожиданный drift private bridge"
    )
    args.output.mkdir(mode=0o700)
    subprocess.run(
        [
            "unshare",
            "--user",
            "--map-root-user",
            "--net",
            "--mount",
            "--pid",
            "--fork",
            "--kill-child=SIGKILL",
            "--mount-proc",
            "sh",
            "-c",
            'mount --make-rprivate / && mount --bind "$1" /mnt && shift && ip link set lo up && exec dbus-run-session -- "$@"',
            "p07",
            str(args.output.resolve()),
            sys.executable,
            str(Path(__file__).resolve()),
            str(Path(args.binary).resolve()),
            str(Path(args.cores).resolve()),
            "/mnt",
            "--inside",
            "--parent-net",
            os.readlink("/proc/self/ns/net"),
        ],
        check=True,
        timeout=250,
    )


if __name__ == "__main__":
    main()
