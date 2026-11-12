"""Один isolated owner snapshot fixture; без host network mutations."""
import json
import os
from pathlib import Path
import resource
import subprocess
import sys
import time


def main() -> int:
    binary = Path(sys.argv[1]).resolve(strict=True)
    scale = int(sys.argv[2])
    if scale not in (1, 10):
        raise ValueError("Допустимы только масштабы 1 и 10")
    environment = dict(os.environ, YONIK_BROKER_ISOLATED="1", YONIK_BROKER_SCALE=str(scale))
    started = time.monotonic()
    result = subprocess.run(
        ["unshare", "--user", "--map-current-user", "--keep-caps", "--net", "--pid", "--fork",
         "--mount-proc", "sh", "-c", 'ip link set lo up && exec "$@"',
         "owner-broker", "setpriv", "--inh-caps=-all", "--ambient-caps=-all",
         str(binary), "--exact", "isolated_snapshot_controls", "--nocapture"],
        env=environment, capture_output=True, text=True, timeout=30, check=False,
    )
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    print(json.dumps({"scale": scale, "exit": result.returncode,
                      "wall_seconds": time.monotonic() - started,
                      "user_seconds": usage.ru_utime, "system_seconds": usage.ru_stime,
                      "max_rss_kib": usage.ru_maxrss,
                      "stdout": result.stdout, "stderr": result.stderr}, ensure_ascii=False))
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
