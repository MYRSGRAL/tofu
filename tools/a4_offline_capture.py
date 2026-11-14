"""Bounded capture собственных offline tools; без запуска команд при импорте."""

from __future__ import annotations

import os
import selectors
import signal
import subprocess
import time

OUTPUT_LIMIT = 1024 * 1024


def capture(
    process: subprocess.Popen[bytes], seconds: float
) -> tuple[str, str, str | None]:
    """Leader остаётся unreaped до killpg, включая нормальное завершение."""
    output = [bytearray(), bytearray()]
    failure = None
    deadline = time.monotonic() + seconds
    streams = (process.stdout, process.stderr)
    try:
        with selectors.DefaultSelector() as selector:
            for index, stream in enumerate(streams):
                if stream is None:
                    raise ValueError("Нужны оба capture pipe")
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, selectors.EVENT_READ, index)
            while selector.get_map() and failure is None:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    failure = "timeout"
                    break
                for key, _ in selector.select(remaining):
                    available = OUTPUT_LIMIT - sum(map(len, output))
                    block = os.read(key.fd, min(65536, available + 1))
                    if not block:
                        selector.unregister(key.fileobj)
                        continue
                    output[key.data].extend(block[:available])
                    if len(block) > available:
                        failure = "overflow"
                        break
            while failure is None:
                # WNOWAIT удерживает PID/PGID до обязательного группового kill.
                status = os.waitid(
                    os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT
                )
                if status is not None:
                    break
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    failure = "timeout"
                    break
                time.sleep(min(0.01, remaining))
    finally:
        try:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            finally:
                try:
                    process.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    failure = "reap_timeout"
        finally:
            for stream in streams:
                if stream is not None:
                    stream.close()
    return (
        output[0].decode("utf-8", errors="replace"),
        output[1].decode("utf-8", errors="replace"),
        failure,
    )
