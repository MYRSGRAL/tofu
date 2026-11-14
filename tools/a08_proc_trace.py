#!/usr/bin/env python3
"""Ptrace только собственного дочернего дерева; сохраняет лишь closed proc классы."""

from __future__ import annotations

import ctypes
import json
import os
import signal
import sys
from pathlib import Path

LIBC = ctypes.CDLL(None, use_errno=True)
LIBC.ptrace.restype = ctypes.c_long
WALL = 0x40000000


def ptrace(request: int, pid: int, address: int = 0, data: int = 0) -> int:
    ctypes.set_errno(0)
    value = LIBC.ptrace(request, pid, ctypes.c_void_p(address), ctypes.c_void_p(data))
    if value == -1 and ctypes.get_errno():
        raise OSError(ctypes.get_errno(), "ptrace")
    return value


def filename(pid: int, address: int) -> str:
    data = bytearray()
    for offset in range(0, 4096, 8):
        word = ptrace(2, pid, address + offset)
        part = (word & ((1 << 64) - 1)).to_bytes(8, "little")
        data.extend(part.split(b"\0", 1)[0])
        if b"\0" in part:
            return os.fsdecode(bytes(data))
    return ""


def main() -> int:
    output = Path(sys.argv[1])
    command = sys.argv[2:]
    child = os.fork()
    if child == 0:
        ptrace(0, 0)
        os.kill(os.getpid(), signal.SIGSTOP)
        os.execvpe(command[0], command, os.environ)
    os.waitpid(child, 0)
    ptrace(0x4200, child, 0, 1 | 2 | 4 | 8 | 0x10 | 0x100000)
    ptrace(24, child)
    active = {child}
    result = 1
    with output.open("w") as log:
        while active:
            pid, status = os.waitpid(-1, WALL)
            if os.WIFEXITED(status) or os.WIFSIGNALED(status):
                active.discard(pid)
                if pid == child:
                    result = (
                        os.WEXITSTATUS(status)
                        if os.WIFEXITED(status)
                        else 128 + os.WTERMSIG(status)
                    )
                continue
            active.add(pid)
            sig = os.WSTOPSIG(status)
            if sig == (signal.SIGTRAP | 0x80):
                info = ctypes.create_string_buffer(128)
                ptrace(0x420E, pid, 128, ctypes.addressof(info))
                raw = info.raw
                if raw[0] == 1:
                    nr = int.from_bytes(raw[24:32], "little")
                    args = [
                        int.from_bytes(raw[i : i + 8], "little")
                        for i in range(32, 80, 8)
                    ]
                    # open/openat/readlink/readlinkat/newfstatat/statx; никаких exec argv.
                    descriptor, address = {
                        2: (-100, args[0]),
                        257: (args[0], args[1]),
                        89: (-100, args[0]),
                        267: (args[0], args[1]),
                        262: (args[0], args[1]),
                        332: (args[0], args[1]),
                    }.get(nr, (-100, 0))
                    if address:
                        try:
                            path = filename(pid, address)
                            if not path.startswith("/") and descriptor not in (
                                -100,
                                (1 << 64) - 100,
                            ):
                                path = (
                                    os.readlink(f"/proc/{pid}/fd/{descriptor}")
                                    + "/"
                                    + path
                                )
                            if path.startswith("/proc/"):
                                parts = path.split("/")
                                leaf = parts[-1]
                                owner = (
                                    "self"
                                    if len(parts) > 2
                                    and parts[2] in ("self", str(pid), "thread-self")
                                    else "candidate"
                                )
                                kind = (
                                    leaf
                                    if leaf
                                    in ("stat", "exe", "status", "cmdline", "environ")
                                    else "other"
                                )
                                log.write(
                                    json.dumps(
                                        {
                                            "tid": pid,
                                            "syscall": nr,
                                            "class": kind,
                                            "owner": owner,
                                        }
                                    )
                                    + "\n"
                                )
                        except OSError as error:
                            log.write(
                                json.dumps(
                                    {
                                        "tid": pid,
                                        "syscall": nr,
                                        "class": "metadata_fd_unresolved"
                                        if nr in (262, 332) and error.errno == 2
                                        else "decode_error",
                                        "errno": error.errno,
                                        "owner": "unknown",
                                    }
                                )
                                + "\n"
                            )
                ptrace(24, pid)
            else:
                ptrace(
                    24, pid, 0, 0 if sig in (signal.SIGTRAP, signal.SIGSTOP) else sig
                )
    return result


if __name__ == "__main__":
    raise SystemExit(main())
