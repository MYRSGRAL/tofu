"""Linux runner готового MSRV test binary; Cargo запускается отдельно через slot."""

import argparse
import hashlib
import json
import os
import resource
import subprocess
from pathlib import Path


def ограничения() -> None:
    resource.setrlimit(resource.RLIMIT_AS, (512 * 1024 * 1024,) * 2)
    resource.setrlimit(resource.RLIMIT_NOFILE, (64, 64))
    resource.setrlimit(resource.RLIMIT_CPU, (60, 60))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    env = os.environ | {"YONIK_ROBUST_CORPUS": str(args.evidence / "corpus.bin")}
    result = subprocess.run(
        [str(args.binary.resolve()), "--test-threads=1"],
        capture_output=True,
        timeout=90,
        env=env,
        preexec_fn=ограничения,
        check=False,
    )
    (args.evidence / "campaign.log").write_bytes(result.stdout + result.stderr)
    summary = {
        "kind": "deterministic mutation campaign, no coverage guidance",
        "seed": "0x921420261112",
        "mutations": 12288,
        "returncode": result.returncode,
        "maxrss_kib": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,
        "address_space_limit_mib": 512,
        "fd_limit": 64,
        "wall_timeout_seconds": 90,
        "cpu_limit_seconds": 60,
        "sha256": {
            str(path): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in [args.binary, Path(__file__), args.evidence / "corpus.bin"]
        },
    }
    (args.evidence / "summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
    )
    if result.returncode:
        raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
