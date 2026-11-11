"""Private FD table в отдельном task; только isolated broker fixture."""
import ctypes
import json
import os
import socket
import sys
import threading


def private_holder() -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.unshare(0x400) != 0:  # CLONE_FILES: FD table этого task становится отдельной.
        raise OSError(ctypes.get_errno(), "unshare(CLONE_FILES)")
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as owned:
        owned.bind(("127.0.0.1", 0))
        owned.connect(("127.0.0.1", 45678))
        if os.environ.get("YONIK_BROKER_NON_DUMPABLE") == "1":
            if libc.prctl(4, 0, 0, 0, 0) != 0:  # PR_SET_DUMPABLE.
                raise OSError(ctypes.get_errno(), "prctl(PR_SET_DUMPABLE)")
        print(json.dumps({"port": owned.getsockname()[1], "tid": threading.get_native_id()}), flush=True)
        sys.stdin.buffer.read(1)


if __name__ == "__main__":
    worker = threading.Thread(target=private_holder)
    worker.start()
    worker.join(timeout=15)
    if worker.is_alive():
        raise TimeoutError("fixture holder не получил завершение")
