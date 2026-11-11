#!/usr/bin/env python3
"""Изолированная bounded приёмка; oracle учитывает все процессы private PID namespace."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from native_cancel_matrix import remaining, snapshot


def inside(args: argparse.Namespace) -> None:
    # Отдельный proc mount обязателен: PID 1 — unshare supervisor, host PID невидимы.
    if Path("/proc/1/ns/pid").readlink() != Path("/proc/self/ns/pid").readlink():
        raise RuntimeError("Нет private proc")
    if os.readlink("/proc/self/ns/net") == args.parent_net:
        raise RuntimeError("Нет private net")
    if sorted(
        line.split(":", 1)[0].strip()
        for line in Path("/proc/net/dev").read_text().splitlines()[2:]
    ) != ["lo"]:
        raise RuntimeError("Ожидался только loopback")
    import socket
    import ssl
    import threading

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(args.output / "cert.pem", args.output / "key.pem")
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    os.environ["HEALTH_TLS_PORT"] = str(listener.getsockname()[1])
    os.environ["SSL_CERT_FILE"] = str(args.output / "ca.pem")
    os.environ["SSL_CERT_DIR"] = str(args.output / "empty-ca")

    def serve() -> None:
        for _ in range(2):
            raw, _ = listener.accept()
            with context.wrap_socket(raw, server_side=True) as stream:
                stream.settimeout(5)
                request = b""
                while b"\r\n\r\n" not in request:
                    chunk = stream.recv(4096)
                    if not chunk:
                        raise RuntimeError("HTTPS запрос оборван")
                    request += chunk
                    if len(request) > 16384:
                        raise RuntimeError("HTTPS request слишком велик")
                stream.sendall(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")

    threading.Thread(target=serve, daemon=True).start()
    baseline = snapshot()
    results = []
    for engine in range(2):
        for axis in ("positive", "cancel", "peer_failure", "desktop_cancel"):
            os.environ["HEALTH_DATA_DIR"] = str(args.output / f"data-{engine}-{axis}")
            samples = []
            seen = set()
            with (args.output / f"{engine}-{axis}.log").open("w") as log:
                process = subprocess.Popen(
                    [args.binary, args.cores, axis, str(engine)], stdout=log, stderr=log
                )
                deadline = time.monotonic() + 20
                while process.poll() is None:
                    rows = remaining(snapshot(), baseline)
                    samples.append(rows)
                    seen.update(rows)
                    if time.monotonic() > deadline:
                        # Popen держит ещё не reaped child: PID reuse исключён.
                        process.kill()
                        process.wait()
                        raise RuntimeError("Deadline")
                    time.sleep(0.001)
                terminal = remaining(snapshot(), baseline)
                record = {
                    "engine": engine,
                    "axis": axis,
                    "exit": process.returncode,
                    "tracked_pid_starttime": sorted(seen),
                    "samples": samples,
                    "terminal_owned_group": terminal,
                }
                results.append(record)
                (args.output / "matrix.json").write_text(json.dumps(results, indent=2))
                if process.returncode or terminal:
                    raise RuntimeError(
                        f"Не принят {engine}/{axis}: {process.returncode}, {terminal}"
                    )
    fixture = subprocess.Popen(
        [
            sys.executable,
            "-c",
            'import time; data=bytearray(8*1024*1024); f=open("/dev/null"); time.sleep(10)',
        ]
    )
    try:
        time.sleep(0.1)
        retained = remaining(snapshot(), baseline)
        if (
            not retained
            or not all((row["fd"] or 0) > 0 for row in retained.values())
            or not any(
                (row["private_dirty"] or 0) >= 8 * 1024 * 1024
                for row in retained.values()
            )
        ):
            raise RuntimeError("Negative oracle пропустил собственный retainer")
        (args.output / "negative.json").write_text(
            json.dumps({"detected": True, "owned_fixture": retained}, indent=2)
        )
    finally:
        fixture.terminate()
        fixture.wait()
    if remaining(snapshot(), baseline):
        raise RuntimeError("Fixture не убран")
    (args.output / "terminal.json").write_text(
        json.dumps(
            {
                "baseline": baseline,
                "terminal": snapshot(),
                "owned_resources_zero": True,
            },
            indent=2,
        )
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary")
    parser.add_argument("cores")
    parser.add_argument("output", type=Path)
    parser.add_argument("--inside", action="store_true")
    parser.add_argument("--parent-net")
    args = parser.parse_args()
    if args.inside:
        inside(args)
        return
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "empty-ca").mkdir()

    def openssl(*parts: str) -> None:
        subprocess.run(
            ["openssl", *parts], check=True, capture_output=True, cwd=args.output
        )

    openssl(
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        "ca-key.pem",
        "-out",
        "ca.pem",
        "-days",
        "1",
        "-subj",
        "/CN=health-fixture-ca",
    )
    openssl(
        "req",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        "key.pem",
        "-out",
        "leaf.csr",
        "-subj",
        "/CN=health-fixture",
    )
    (args.output / "extensions").write_text(
        "basicConstraints=critical,CA:FALSE\nsubjectAltName=IP:198.18.0.1\nextendedKeyUsage=serverAuth\n"
    )
    openssl(
        "x509",
        "-req",
        "-in",
        "leaf.csr",
        "-CA",
        "ca.pem",
        "-CAkey",
        "ca-key.pem",
        "-CAcreateserial",
        "-out",
        "cert.pem",
        "-days",
        "1",
        "-extfile",
        "extensions",
    )
    manifest = {
        "source_head": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], text=True
        ).strip(),
        "sha256": {},
    }
    for path in [
        Path(args.binary),
        Path(__file__),
        Path("tools/native_cancel_matrix.py"),
        Path("crates/yonik-desktop/examples/health_probe_cancel.rs"),
        *Path(args.cores).glob("*/*"),
    ]:
        if path.is_file():
            manifest["sha256"][str(path)] = hashlib.sha256(
                path.read_bytes()
            ).hexdigest()
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    command = [
        "unshare",
        "--user",
        "--map-root-user",
        "--net",
        "--pid",
        "--fork",
        "--kill-child=SIGKILL",
        "--mount-proc",
        "sh",
        "-c",
        'ip link set lo up && exec dbus-run-session -- "$@"',
        "matrix",
        sys.executable,
        str(Path(__file__).resolve()),
        args.binary,
        args.cores,
        str(args.output.resolve()),
        "--inside",
        "--parent-net",
        os.readlink("/proc/self/ns/net"),
    ]
    subprocess.run(command, check=True, timeout=190)


if __name__ == "__main__":
    main()
