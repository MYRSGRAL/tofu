"""Пересчёт разброса принятых P04 runs без повторного benchmark."""

import hashlib
import json
import statistics
import subprocess
from pathlib import Path


def summarize(rows: list[dict]) -> list[dict]:
    """Три успешных measured runs на комбинацию; warmup не учитывается."""
    groups: dict[tuple, list[dict]] = {}
    for row in rows:
        if not row["warmup"]:
            key = (row["count"], row.get("rules"), row.get("engine"))
            groups.setdefault(key, []).append(row)
    output = []
    for key, samples in groups.items():
        if len(samples) != 3 or any(s["cancelled"] or s["exit"] != 0 for s in samples):
            raise ValueError("Нужны ровно три успешных measured runs")
        metrics = {}
        for metric in ("wall_s", "cpu_s", "peak_rss_kib"):
            values = [s[metric] for s in samples]
            mean = statistics.mean(values)
            metrics[metric] = {
                "min": min(values),
                "max": max(values),
                "median": statistics.median(values),
                "sample_variance": statistics.variance(values),
                "cv_percent": statistics.stdev(values) / mean * 100 if mean else 0,
            }
        output.append({"count_rules_engine": key, "repetitions": 3, "metrics": metrics})
    if not output:
        raise ValueError("Нет measured runs")
    return output


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    paths = [
        "tools/perf_import_compile.py",
        "crates/yonik-engine/examples/import_compile_resource.rs",
        "crates/yonik-engine/src/resource_bundle.rs",
        "crates/yonik-engine/src/routing.rs",
        "crates/yonik-engine/src/policy_input.rs",
    ]
    report = {
        "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root)
        .decode()
        .strip(),
        "status": "OPEN: positive mixed compiler, cancellation and recovery not measured",
        "sources_sha256": {
            p: hashlib.sha256((root / p).read_bytes()).hexdigest() for p in paths
        },
        "campaigns": {},
    }
    for campaign in ("p04-import-compile", "p04-subscription-snapshots"):
        folder = root / "docs/validation" / campaign
        raw = (folder / "runs.json").read_bytes()
        report["campaigns"][campaign] = {
            "runs_sha256": hashlib.sha256(raw).hexdigest(),
            "manifest_sha256": hashlib.sha256(
                (folder / "manifest.json").read_bytes()
            ).hexdigest(),
            "summary": summarize(json.loads(raw)),
        }
    print(json.dumps(report, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
