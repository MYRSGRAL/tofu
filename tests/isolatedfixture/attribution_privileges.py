"""Отрицательное доказательство foreign-UID proc lookup; host не изменяется."""

from __future__ import annotations

import ctypes
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path


def enter_worker() -> None:
    """Оставить ровно CAP_NET_ADMIN после перехода на отдельный UID."""
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(8, 1, 0, 0, 0) != 0:  # PR_SET_KEEPCAPS
        raise OSError(ctypes.get_errno(), "prctl")
    os.setgroups([])
    os.setgid(1002)
    os.setuid(1002)
    header = (ctypes.c_uint32 * 2)(0x20080522, 0)
    data = (ctypes.c_uint32 * 6)(0x1000, 0x1000, 0, 0, 0, 0)
    if libc.capset(ctypes.byref(header), ctypes.byref(data)) != 0:
        raise OSError(ctypes.get_errno(), "capset")
    if libc.prctl(38, 1, 0, 0, 0) != 0:  # PR_SET_NO_NEW_PRIVS
        raise OSError(ctypes.get_errno(), "prctl")


def guest() -> None:
    uid_map = Path("/proc/self/uid_map").read_text()
    assert len(uid_map.splitlines()) == 2
    assert any(int(row.split()[2]) > 1002 for row in uid_map.splitlines())
    assert Path("/proc/self/ns/net").stat().st_ino != int(sys.argv[2])
    links = json.loads(subprocess.check_output(["/usr/bin/ip", "-j", "link"]))
    assert {link["ifname"] for link in links} == {"lo"}
    subprocess.run(["/usr/bin/ip", "link", "set", "lo", "up"], check=True)
    proof_r, proof_w = os.pipe()
    helper = os.fork()
    if helper == 0:
        os.close(proof_r)
        libc = ctypes.CDLL(None, use_errno=True)
        header = (ctypes.c_uint32 * 2)(0x20080522, 0)
        data = (ctypes.c_uint32 * 6)(0x1000, 0x1000, 0, 0, 0, 0)
        assert libc.capset(ctypes.byref(header), ctypes.byref(data)) == 0
        try:
            subprocess.run(
                [sys.executable, "-c", "pass"],
                user=1002,
                group=1002,
                extra_groups=[],
                check=True,
            )
        except OSError as error:
            os.write(proof_w, str(error.errno).encode())
        os._exit(0)
    os.close(proof_w)
    helper_spawn_errno = int(os.read(proof_r, 32))
    os.close(proof_r)
    os.waitpid(helper, 0)
    assert helper_spawn_errno == 1  # EPERM: ни SETUID, ни SETGID в helper unit.
    ready_r, ready_w = os.pipe()
    hold_r, hold_w = os.pipe()
    client = os.fork()
    if client == 0:
        os.close(ready_r)
        os.close(hold_w)
        os.setgroups([])
        os.setgid(1001)
        os.setuid(1001)
        sockets = []
        rows = []
        for family, address in [
            (socket.AF_INET, "127.0.0.1"),
            (socket.AF_INET6, "::1"),
        ]:
            for kind in [socket.SOCK_STREAM, socket.SOCK_DGRAM]:
                scale = int(os.environ.get("YONIK_ATTRIBUTION_SCALE", "1"))
                assert scale in {1, 10}
                for _ in range(scale):
                    s = socket.socket(family, kind)
                    s.bind((address, 0))
                    if kind == socket.SOCK_STREAM:
                        s.listen()
                    sockets.append(s)
                    rows.append(
                        {
                            "family": int(family),
                            "kind": int(kind),
                            "inode": os.fstat(s.fileno()).st_ino,
                        }
                    )
        os.write(ready_w, json.dumps(rows).encode())
        os.close(ready_w)
        os.read(hold_r, 1)
        os._exit(0)
    os.close(ready_w)
    os.close(hold_r)
    rows = json.loads(os.read(ready_r, 65536))
    os.close(ready_r)
    worker = os.fork()
    if worker == 0:
        os.close(hold_w)
        enter_worker()
        status = Path("/proc/self/status").read_text()
        assert "CapEff:\t0000000000001000" in status
        failures = {}
        for name in ["exe", "fd", "ns/net"]:
            try:
                path = Path(f"/proc/{client}/{name}")
                list(path.iterdir()) if name == "fd" else path.readlink()
            except OSError as error:
                failures[name] = error.errno
        assert failures == {"exe": 13, "fd": 13, "ns/net": 13}, failures
        kernel_rows = {}
        for name in ["tcp", "udp", "tcp6", "udp6"]:
            lines = Path(f"/proc/net/{name}").read_text().splitlines()
            owned = [
                line.split()
                for line in lines[1:]
                if int(line.split()[9]) in {row["inode"] for row in rows}
            ]
            assert owned and all(int(row[7]) == 1001 for row in owned)
            kernel_rows[name] = {
                "header": lines[0],
                "matched_rows": len(owned),
                "uid": 1001,
            }
        assert sum(value["matched_rows"] for value in kernel_rows.values()) == len(rows)
        costs = []
        for count in [1, 10]:
            start = time.monotonic_ns()
            total = 0
            for _ in range(count):
                for name in ["tcp", "udp", "tcp6", "udp6"]:
                    lines = Path(f"/proc/net/{name}").read_text().splitlines()
                    assert "local_address" in lines[0]
                    total += len(lines) - 1
            costs.append(
                {
                    "lookups": count,
                    "rows": total,
                    "elapsed_ns": time.monotonic_ns() - start,
                }
            )
        result = {
            "uid_map": uid_map,
            "kernel": os.uname().release,
            "worker_status": status,
            "client_uid": 1001,
            "worker_uid": os.getuid(),
            "client_sockets": rows,
            "kernel_owner_rows": kernel_rows,
            "denied_errno": failures,
            "helper_setuid_child_errno": helper_spawn_errno,
            "scan_cost": costs,
            "cgroup": Path("/proc/self/cgroup").read_text(),
            "mountinfo": Path("/proc/self/mountinfo").read_text(),
            "limits": "Kernel negative proof; не systemd/native routing gate, не G-POWER. FD lookup запрещён, его стоимость не измерена.",
        }
        sys.stdout.write(json.dumps(result, ensure_ascii=False) + "\n")
        sys.stdout.flush()
        os._exit(0)
    try:
        _, status = os.waitpid(worker, 0)
        assert os.waitstatus_to_exitcode(status) == 0
    finally:
        os.close(hold_w)
        os.waitpid(client, 0)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--guest":
        guest()
    else:
        subprocess.run(
            [
                "/usr/bin/unshare",
                "--user",
                "--map-auto",
                "--map-root-user",
                "--net",
                "--pid",
                "--fork",
                "--mount-proc",
                sys.executable,
                str(Path(__file__).resolve()),
                "--guest",
                str(Path("/proc/self/ns/net").stat().st_ino),
            ],
            check=True,
        )
