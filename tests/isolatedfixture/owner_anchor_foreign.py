"""Actual retained nsfs FD proof после helper UID drop; admission всегда proof-gap."""
import array
import hashlib
import fcntl
import json
import os
import select
import socket
import struct
import subprocess
import sys

WORKER = r'''
import array,os,socket,struct,sys
s=socket.socket(fileno=int(sys.argv[1]))
s.setsockopt(socket.SOL_SOCKET,socket.SO_PASSCRED,1)
frame,anc,flags,_=s.recvmsg(76,socket.CMSG_SPACE(12))
credentials=[struct.unpack("3i",b) for level,kind,b in anc if level==socket.SOL_SOCKET and kind==socket.SCM_CREDENTIALS]
assert credentials==[(os.getppid(),0,0)],credentials
assert not flags and len(frame)==76
fds=[os.open('/proc/self/ns/'+n,os.O_RDONLY|os.O_CLOEXEC) for n in ('net','pid','mnt')]
s.sendmsg([frame],[(socket.SOL_SOCKET,socket.SCM_RIGHTS,array.array('i',fds))])
s.recv(1)
'''


def credentials(ancillary: list) -> tuple[int, int, int]:
    values = [struct.unpack("3i", data) for level, kind, data in ancillary
              if level == socket.SOL_SOCKET and kind == socket.SCM_CREDENTIALS]
    if len(values) != 1:
        raise ValueError("Нет единственных kernel credentials")
    return values[0]


def main() -> int:
    assert os.getuid() == 0
    mapping = open("/proc/self/uid_map", encoding="ascii").read().split()
    assert mapping[1] != "0", "Запрещено на host root"
    rx, tx = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    rx.setsockopt(socket.SOL_SOCKET, socket.SO_PASSCRED, 1)
    rx.settimeout(3)

    def drop_worker() -> None:
        os.setgroups([])
        os.setgid(1001)
        os.setuid(1001)

    child = subprocess.Popen([sys.executable, "-c", WORKER, str(tx.fileno())],
                             pass_fds=(tx.fileno(),), preexec_fn=drop_worker)
    tx.close()
    held = []
    expected = []
    pidfd = os.pidfd_open(child.pid)
    try:
        initial_stat = open(f"/proc/{child.pid}/stat", encoding="ascii").read().rsplit(")", 1)[1].split()[19]
        initial_cgroup = open(f"/proc/{child.pid}/cgroup", encoding="ascii").read()
        # Root host constructor capture из fixed child, до отправки challenge.
        for name in ("net", "pid", "mnt"):
            expected.append(os.open(f"/proc/{child.pid}/ns/{name}", os.O_RDONLY | os.O_CLOEXEC))
        identities = [(os.fstat(fd).st_dev, os.fstat(fd).st_ino) for fd in expected]
        frame = struct.pack(">4sQ32s32s", b"TNS1", 7, os.urandom(32), hashlib.sha256(b"fixture-config").digest())
        rx.send(frame)
        message, ancillary, flags, _ = rx.recvmsg(77, socket.CMSG_SPACE(12) + socket.CMSG_SPACE(12), socket.MSG_CMSG_CLOEXEC)
        assert not flags & (socket.MSG_TRUNC | socket.MSG_CTRUNC) and message == frame
        assert credentials(ancillary) == (child.pid, 1001, 1001)
        for level, kind, data in ancillary:
            if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                values = array.array("i")
                values.frombytes(data)
                held.extend(values)
        assert len(held) == 3
        assert [(os.fstat(fd).st_dev, os.fstat(fd).st_ino) for fd in held] == identities
        for fd in expected:
            os.close(fd)
        expected.clear()
        os.setgroups([])
        os.setgid(1000)
        os.setuid(1000)
        assert os.getuid() == os.geteuid() == 1000
        status = open("/proc/self/status", encoding="ascii").read()
        for key in ("CapInh", "CapPrm", "CapEff", "CapAmb"):
            assert int(next(line.split()[1] for line in status.splitlines() if line.startswith(key + ":")), 16) == 0
        assert [(os.fstat(fd).st_dev, os.fstat(fd).st_ino) for fd in held] == identities
        # UAPI linux/nsfs.h NS_GET_NSTYPE; ioctl доступен без ptrace/ns reopening.
        assert [fcntl.ioctl(fd, 0xB703) for fd in held] == [0x40000000, 0x20000000, 0x20000]
        try:
            fd = os.open(f"/proc/{child.pid}/ns/net", os.O_RDONLY)
        except PermissionError:
            direct = "WorkerViewUnavailable"
        else:
            os.close(fd)
            raise AssertionError("Ожидался foreign proc denial")
        assert open(f"/proc/{child.pid}/stat", encoding="ascii").read().rsplit(")", 1)[1].split()[19] == initial_stat
        assert open(f"/proc/{child.pid}/cgroup", encoding="ascii").read() == initial_cgroup
        info = open(f"/proc/self/fdinfo/{pidfd}", encoding="ascii").read()
        assert f"Pid:\t{child.pid}\n" in info
        assert not select.select([pidfd], [], [], 0)[0]
        rx.send(b"x")
        assert child.wait(timeout=3) == 0
        assert select.select([pidfd], [], [], 1)[0]
        print(json.dumps({"helper_uid_after_drop": os.getuid(), "worker_uid": 1001,
                          "kernel_sender_authenticated": True, "retained_namespace_fds": 3, "namespace_types_verified": True,
                          "direct_proc": direct, "admission": "UnitProofGap",
                          "pidfd_death": True, "unit_immutability": "not attested"}))
    finally:
        rx.close()
        for fd in held + expected + [pidfd]:
            os.close(fd)
        if child.poll() is None:
            # После UID drop kill чужого UID запрещён: EOF завершает worker.
            child.wait(timeout=3)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
