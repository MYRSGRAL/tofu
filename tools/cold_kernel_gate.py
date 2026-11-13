"""Запуск retained test binary в собственных namespaces без host /proc campaign."""

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    args.evidence.mkdir(parents=True, exist_ok=True)
    results = []
    tests = [
        ("cold_tests::isolated_cold_wire_known", True),
        ("cold_tests::isolated_cold_limit_cancel", True),
        ("tests::actual_udp_reuseport_dump_preserves_ambiguity", False),
        ("tests::cancellation_during_actual_capture_releases_partial_proof", False),
        ("tests::retained_projection_positive_and_stale_replaced_deleted_epoch", False),
    ]
    for suffix, ignored in tests:
        test = f"attribution::broker::kernel::{suffix}"
        command = [
            "unshare",
            "--user",
            "--map-current-user",
            "--keep-caps",
            "--mount",
            "--pid",
            "--fork",
            "--mount-proc",
            "--net",
            "sh",
            "-ec",
            'ip link set lo up; exec setpriv --inh-caps=-all --ambient-caps=-all "$@"',
            "cold-gate",
            str(binary),
            "--exact",
            test,
            "--nocapture",
            "--test-threads=1",
        ]
        if ignored:
            command.append("--ignored")
        result = subprocess.run(
            command,
            capture_output=True,
            timeout=30,
            check=False,
            env={**os.environ, "YONIK_BROKER_GATES": "1"},
        )
        output = result.stdout + result.stderr
        (args.evidence / f"{suffix}.log").write_bytes(output)
        passed = result.returncode == 0 and b"1 passed" in output
        results.append(
            {
                "test": test,
                "command": command,
                "returncode": result.returncode,
                "passed": passed,
            }
        )
    report = {
        "binary": str(binary),
        "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "uid": os.getuid(),
        "results": results,
    }
    (args.evidence / "run.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if all(row["passed"] for row in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
