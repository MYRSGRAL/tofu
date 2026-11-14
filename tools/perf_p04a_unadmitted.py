"""Targeted mixed data campaign: прежние import/snapshot runs не повторяются."""

import argparse
import hashlib
import json
import statistics
import subprocess
from pathlib import Path

from perf_subscription_snapshots import run_child


def validate(row: dict, rules: int) -> None:
    """Отказ отделён от COMPILED, cancellation/recovery обязаны быть измерены."""
    if row.get("cancelled") is not False or row.get("exit") != 0:
        raise ValueError("Child не завершён")
    result = row["result"]
    if (result["rules"], result["domains"], result["cidrs"]) != (rules, 100000, 50000):
        raise ValueError("Неполная нагрузка")
    rounds = result["rounds"]
    if len(rounds) != 4:
        raise ValueError("Неполные recovery rounds")
    for item in rounds:
        if item["status"] == "REFUSED":
            if rules != 10000 or item["bytes"] != 0:
                raise ValueError("Неожиданный отказ")
        elif item["status"] != "COMPILED" or not 0 < item["bytes"] <= 4194304:
            raise ValueError("Неверный output budget/status")
        if item["status"] == "COMPILED" and (
            item["native_rules"] != rules + 3
            or not rules + 3 <= item["native_nodes"] <= 5 * rules + 16
        ):
            raise ValueError("Неограниченное раскрытие правил")
        if len(item["cancel"]) != 3 or any(c["ns"] <= 0 for c in item["cancel"]):
            raise ValueError("Нет actual cancellation")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("fixture_root", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    paths = (
        subprocess.check_output(
            ["git", "ls-files", "crates", "Cargo.lock", "Cargo.toml"]
        )
        .decode()
        .splitlines()
    )
    paths += (
        [str(Path(__file__).relative_to(Path.cwd()))]
        if Path(__file__).is_absolute()
        else [__file__]
    )
    paths += [
        "tools/perf_subscription_snapshots.py",
        "crates/yonik-linux/examples/p04a_unadmitted_resource.rs",
        "crates/yonik-linux/examples/p04a_unadmitted_data.rs",
    ]
    manifest = {
        "head": subprocess.check_output(["git", "rev-parse", "HEAD"]).decode().strip(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "sources_sha256": {
            p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
            for p in sorted(set(paths))
        },
        "scope": "mixed compile-only, 100k domains/50k CIDR; old server/subscription baseline reused",
    }
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    rows = []
    summary = []
    for engine in ("xray", "sing-box"):
        for rules in (10, 100, 1000, 10000):
            samples = []
            for repeat in range(4):
                path = args.fixture_root / f"{engine}-{rules}-{repeat}"
                row = {
                    "engine": engine,
                    "rules": rules,
                    "repeat": repeat,
                    "warmup": repeat == 0,
                    **run_child([str(binary), engine, str(rules), str(path)], b"", 60),
                }
                validate(row, rules)
                rows.append(row)
                (args.output / "runs.json").write_text(
                    json.dumps(rows, indent=2) + "\n"
                )
                if repeat:
                    samples.append(row)
            wall = [s["wall_s"] for s in samples]
            round_rows = [r for s in samples for r in s["result"]["rounds"]]
            compile_ns = [
                statistics.median(r["compile_ns"] for r in sample["result"]["rounds"])
                for sample in samples
            ]
            plateaus = [
                max(r["rss_after_recovery_kib"] for r in s["result"]["rounds"][1:])
                - min(r["rss_after_recovery_kib"] for r in s["result"]["rounds"][1:])
                for s in samples
            ]
            summary.append(
                {
                    "engine": engine,
                    "rules": rules,
                    "repetitions": 3,
                    "statuses": sorted({r["status"] for r in round_rows}),
                    "wall_median_s": statistics.median(wall),
                    "wall_sample_variance": statistics.variance(wall),
                    "wall_cv_percent": statistics.stdev(wall)
                    / statistics.mean(wall)
                    * 100,
                    "compile_median_ns": statistics.median(compile_ns),
                    "compile_sample_variance_ns2": statistics.variance(compile_ns),
                    "compile_cv_percent": statistics.stdev(compile_ns)
                    / statistics.mean(compile_ns)
                    * 100,
                    "max_peak_rss_kib": max(s["peak_rss_kib"] for s in samples),
                    "max_output_bytes": max(r["bytes"] for r in round_rows),
                    "max_cancel_ns": max(
                        c["ns"] for r in round_rows for c in r["cancel"]
                    ),
                    "max_cancel_ack_ns": max(
                        c["request_to_return_ns"]
                        for r in round_rows
                        for c in r["cancel"]
                    ),
                    "max_recovery_rss_range_kib": max(plateaus),
                    "recovery_8mib_engineering_budget_met": max(plateaus) <= 8192,
                }
            )
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
