"""Native negative gate в собственной VM: разные UID, NET_ADMIN, TCP/UDP v4/v6."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import attribution_gate as gate


def command(uid: int, gid: int, capabilities: bool) -> list[str]:
    capability = "+net_admin" if capabilities else "-all"
    return [
        "/usr/bin/setpriv",
        f"--reuid={uid}",
        f"--regid={gid}",
        "--clear-groups",
        f"--inh-caps={capability}",
        f"--ambient-caps={capability}",
        "--bounding-set=-all,+net_admin" if capabilities else "--bounding-set=-all",
        "--no-new-privs",
        "--",
    ]


def guest(binary: Path, engine: str, output: Path, host: int, scale: int) -> None:
    if os.stat("/proc/self/ns/net").st_ino == host or gate.links() != {"lo"}:
        raise RuntimeError("Требуется собственный пустой netns")
    import pwd

    worker = pwd.getpwnam("yonik-worker")
    client = pwd.getpwnam("yonik-vm-allowed")
    assert worker.pw_uid != client.pw_uid and client.pw_uid != 0
    gate.ip("link", "set", "lo", "up")
    with tempfile.TemporaryDirectory(prefix="yonik-native-foreign-") as directory:
        work = Path(directory)
        work.chmod(0o755)
        executables = []
        for name in ["a", "b"]:
            (work / name).mkdir(mode=0o755)
            path = work / name / "client"
            shutil.copy2(Path(sys.executable).resolve(), path)
            path.chmod(0o755)
            executables.append(path)
        config = work / "tun.json"
        gate.write_json(config, gate.config(engine, str(executables[0])))
        config.chmod(0o644)
        peer = work / "peer.json"
        gate.write_json(peer, gate.marker_peer(engine))
        for name, path in [("config", config), ("peer", peer)]:
            shutil.copy2(path, output / f"{engine}-{scale}x-{name}.json")
        gate.write_json(
            output / f"{engine}-{scale}x-manifest-before.json",
            {
                "worker_uid": worker.pw_uid,
                "client_uid": client.pw_uid,
                "binary_sha256": hashlib.file_digest(
                    binary.open("rb"), "sha256"
                ).hexdigest(),
                "config_sha256": hashlib.sha256(config.read_bytes()).hexdigest(),
                "peer_sha256": hashlib.sha256(peer.read_bytes()).hexdigest(),
                "fixture_sha256": hashlib.sha256(
                    Path(__file__).read_bytes()
                ).hexdigest(),
                "production_unit_sha256": hashlib.sha256(
                    Path("/usr/lib/systemd/system/yonik-worker.service").read_bytes()
                ).hexdigest(),
            },
        )
        rows = []
        with (
            gate.markers(),
            gate.core(binary, peer, 24443),
            (output / f"{engine}-{scale}x.log").open("wb") as log,
        ):
            process = subprocess.Popen(
                command(worker.pw_uid, worker.pw_gid, True)
                + [str(binary), "run", "-c", str(config)],
                stdout=log,
                stderr=log,
            )
            try:
                deadline = time.monotonic() + 8
                while "yonik-test" not in gate.links():
                    if process.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError("Native worker не создал TUN")
                    time.sleep(0.05)
                status = Path(f"/proc/{process.pid}/status").read_text()
                assert "CapEff:\t0000000000001000" in status
                gate.ip("address", "replace", "198.18.0.1/29", "dev", "yonik-test")
                gate.ip(
                    "-6",
                    "address",
                    "replace",
                    "2001:db8:746f::1/125",
                    "dev",
                    "yonik-test",
                )
                gate.ip("link", "set", "yonik-test", "up")
                for address in ["198.18.0.4/32", "2001:db8:746f::4/128"]:
                    gate.ip(
                        *(["-6"] if ":" in address else []),
                        "route",
                        "replace",
                        address,
                        "dev",
                        "yonik-test",
                    )
                for _ in range(scale):
                    for family, prefix in [
                        (socket.AF_INET, "198.18.0."),
                        (socket.AF_INET6, "2001:db8:746f::"),
                    ]:
                        for kind in [socket.SOCK_STREAM, socket.SOCK_DGRAM]:
                            for suffix in [2, 4]:
                                address = prefix + str(suffix)
                                route = json.loads(
                                    subprocess.check_output(
                                        ["ip", "-j", "route", "get", address]
                                    )
                                )
                                assert route[0]["dev"] == "yonik-test"
                                for executable in executables:
                                    result = subprocess.run(
                                        command(client.pw_uid, client.pw_gid, False)
                                        + [
                                            str(executable),
                                            "-c",
                                            gate.CLIENT,
                                            str(int(family)),
                                            str(int(kind)),
                                            address,
                                            "0",
                                        ],
                                        capture_output=True,
                                        text=True,
                                        check=True,
                                        timeout=4,
                                    )
                                    value = json.loads(result.stdout)
                                    rows.append(
                                        {
                                            "family": int(family),
                                            "kind": int(kind),
                                            "destination": address,
                                            "selected": executable == executables[0],
                                            **value,
                                        }
                                    )
                report = {
                    "engine": engine,
                    "scale": scale,
                    "worker_uid": worker.pw_uid,
                    "client_uid": client.pw_uid,
                    "worker_status": status,
                    "binary_sha256": hashlib.file_digest(
                        binary.open("rb"), "sha256"
                    ).hexdigest(),
                    "rows": rows,
                    "all_default": all(row["marker"] == "D" for row in rows),
                    "limits": "Отрицательный native witness с UID/caps production; без systemd mount sandbox. Не положительный FR20 и не G-POWER; elapsed_us включает весь flow, не отдельный procFD lookup.",
                }
                gate.write_json(output / f"{engine}-{scale}x.json", report)
                assert report["all_default"], report
            finally:
                gate.stop(process)


if __name__ == "__main__":
    assert (
        Path("/etc/yonik-production-helper-vm").read_text()
        == "yonik-production-helper-disposable-v1\n"
    )
    assert Path("/sys/class/dmi/id/sys_vendor").read_text().strip() == "QEMU"
    if sys.argv[1] == "--guest":
        guest(
            Path(sys.argv[2]),
            sys.argv[3],
            Path(sys.argv[4]),
            int(sys.argv[5]),
            int(sys.argv[6]),
        )
    else:
        binary, engine, output = (
            Path(sys.argv[1]).resolve(),
            sys.argv[2],
            Path(sys.argv[3]).resolve(),
        )
        scale = int(sys.argv[4])
        assert scale in {1, 10}
        output.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [
                "/usr/bin/unshare",
                "--net",
                "--pid",
                "--fork",
                "--mount-proc",
                sys.executable,
                str(Path(__file__).resolve()),
                "--guest",
                str(binary),
                engine,
                str(output),
                str(os.stat("/proc/self/ns/net").st_ino),
                str(scale),
            ],
            check=True,
            timeout=180,
        )
