"""Контролируемое fork/exec окно для directory flock с CLOEXEC, без host I/O."""
import errno
import fcntl
import os
import signal
import sys

signal.alarm(5)
path = sys.argv[1]  # Только собственный приватный каталог Rust fixture.
fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
ready_r, ready_w = os.pipe()
release_r, release_w = os.pipe()
pid = os.fork()
if pid == 0:
    os.close(ready_r)
    os.close(release_w)
    os.fstat(fd)  # CLOEXEC не закрывает FD при fork.
    os.write(ready_w, b"F")
    if os.read(release_r, 1) != b"E":
        os._exit(2)
    os.execv("/bin/true", ["true"])

os.close(ready_w)
os.close(release_r)
try:
    assert os.read(ready_r, 1) == b"F"
    os.close(fd)  # Последний parent FD закрыт; child пока удерживает тот же OFD.
    contender = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError as error:
        assert error.errno in (errno.EAGAIN, errno.EACCES)
        print("fork_before_exec: inherited OFD retains directory flock", flush=True)
    else:
        raise AssertionError("fork witness отсутствует")
    os.write(release_w, b"E")
    _, status = os.waitpid(pid, 0)
    pid = None
    assert status == 0
    fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.close(contender)
    print("after_exec: CLOEXEC releases last OFD and contender acquires", flush=True)
finally:
    if pid is not None:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
    os.close(ready_r)
    os.close(release_w)
