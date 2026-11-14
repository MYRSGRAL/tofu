"""Focused P08 proof: импорт → compile → exact JSON → pinned config-check."""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import subprocess
from pathlib import Path

from engine_compile import check_ss2022


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--xray", type=Path, required=True)
    parser.add_argument("--sing-box", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    records = {}
    for engine, binary, expected in [
        ("xray", args.xray, "26.10.31"),
        ("sing-box", args.sing_box, "1.14.4"),
    ]:
        version = subprocess.run(
            [str(binary), "version"], capture_output=True, check=True, timeout=5
        ).stdout.decode()
        if expected not in version.splitlines()[0]:
            raise RuntimeError(f"Неверная версия {engine}: {version}")
        records[engine] = {
            "path": str(binary),
            "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "version": version,
        }
        check_ss2022(args.compiler, binary, engine, args.evidence)
    records["compiler"] = {
        "path": str(args.compiler),
        "sha256": hashlib.sha256(args.compiler.read_bytes()).hexdigest(),
    }
    (args.evidence / "artifacts.json").write_text(json.dumps(records, indent=2) + "\n")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    main()
