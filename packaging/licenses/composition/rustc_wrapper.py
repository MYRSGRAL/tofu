#!/usr/bin/env python3
"""Additive instrumentation: invocation log и map только финальных bin links."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path


def main() -> None:
    compiler, *args = sys.argv[1:]
    root = Path(os.environ["YONIK_COMPOSITION_EVIDENCE"])
    name = args[args.index("--crate-name") + 1] if "--crate-name" in args else "probe"
    types = args[args.index("--crate-type") + 1] if "--crate-type" in args else ""
    addition = []
    if types == "bin" and name in {"yonik_desktop", "yonik_helper", "yonik_worker"}:
        addition = ["-C", f"link-arg=-Wl,-Map={root}/{name}.map"]
    record = {
        "compiler": compiler,
        "crate": name,
        "crate_type": types,
        "args": args,
        "instrumentation": addition,
    }
    # jobs=1, append также сохраняет границы записей для compiler probes.
    with (root / "rustc-invocations.jsonl").open("a") as output:
        output.write(json.dumps(record, ensure_ascii=False) + "\n")
    sys.exit(subprocess.call([compiler, *args, *addition]))


if __name__ == "__main__":
    main()
