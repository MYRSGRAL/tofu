"""Один собственный namespace fixture для broker gates, включая root-marked UID checks."""
import json
import os
from pathlib import Path
import resource
import subprocess
import sys
import tempfile
import time


def main() -> int:
    binary = Path(sys.argv[1]).resolve(strict=True)
    mode = sys.argv[2]
    if mode not in ("app", "rootmarked"):
        raise ValueError("Режим: app или rootmarked")
    environment = dict(os.environ, YONIK_BROKER_GATES="1")
    base = Path(__file__).resolve().parent
    environment["YONIK_BROKER_THREAD_FIXTURE"] = str(base / "owner_broker_threads.py")
    environment["YONIK_BROKER_EXEC_FIXTURE"] = str(base / "owner_broker_exec.py")
    with tempfile.TemporaryDirectory(prefix="owner-broker-uid-mount-") as temporary:
        mountpoint = Path(temporary)
        mountpoint.chmod(0o755)
        namespace = ["unshare", "--user", "--net", "--pid", "--fork", "--kill-child", "--mount-proc", "--propagation", "private"]
        if mode == "rootmarked":
            environment["YONIK_BROKER_ROOTMARKED"] = "1"
            # Только root нового user namespace. Bind существует лишь в новом mount namespace.
            command = namespace + ["--map-auto", "--map-root-user", "sh", "-c",
                'mount --bind "$1" "$2" && cd "$2" && exec ./uid-libtest --exact attribution::broker::receiver_tests::actual_foreign_uid_credentials_and_admission --nocapture',
                "owner-broker", str(binary.parent), str(mountpoint)]
        else:
            command = namespace + ["--map-current-user", "--keep-caps", "sh", "-c",
                'ip link set lo up && exec "$@"', "owner-broker", "setpriv", "--inh-caps=-all", "--ambient-caps=-all",
                str(binary), *sys.argv[3:], "--test-threads=1", "--nocapture"]
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        started = time.monotonic()
        result = subprocess.run(command, env=environment, capture_output=True, text=True, timeout=30, check=False)
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        print(json.dumps({"mode": mode, "exit": result.returncode,
            "wall_seconds": time.monotonic() - started, "user_seconds": usage.ru_utime - before.ru_utime,
            "system_seconds": usage.ru_stime - before.ru_stime, "max_rss_kib": usage.ru_maxrss,
            "stdout": result.stdout, "stderr": result.stderr}, ensure_ascii=False))
        return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
