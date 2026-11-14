#!/usr/bin/env python3
"""Проверить frozen raw P-03 результаты и извлечь evidence без нормализации памяти."""

from __future__ import annotations

import argparse
import json
import math
import re
from pathlib import Path

PHASES = (
    "rules_ready_frame",
    "advanced_ready_frame",
    "simple_ready_frame",
    "reopen_ready_frame",
)


def percentile(values: list[float]) -> float:
    return sorted(values)[math.ceil(len(values) * 0.95) - 1]


def convert(record: dict) -> dict:
    result = dict(record)
    memory = result.pop("smaps_rollup")
    for label, key in [
        ("Pss", "pss_kib"),
        ("Private_Dirty", "private_dirty_kib"),
        ("Pss_Anon", "pss_anon_kib"),
        ("Pss_File", "pss_file_kib"),
    ]:
        result[key] = int(re.search(r"^" + label + r":\s+(\d+)", memory, re.M)[1])
    return result


def report(base: Path, output: Path) -> None:
    raw = json.loads((base / "final/results.json").read_text())
    preflight = json.loads((base / "preflight-1/results.json").read_text())
    model = json.loads((base / "model-preflight/results.json").read_text())
    assert len(raw["runs"]) == 3
    runs = []
    for run in raw["runs"]:
        assert run["exit"] == 0 and run["records"][-1]["phase"] == "PASS"
        records = [convert(r) for r in run["records"]]
        seed = next(r for r in records if r["phase"] == "seeded")
        assert len(seed["routing"]["rules"]) == 2
        mixed = seed["routing"]["rules"][0]
        assert all(
            len(mixed[k]) == 2 for k in ("executables", "domains", "destinations")
        )
        checks = [r for r in records if r["phase"] == "policy_and_fields"]
        assert len(checks) == 200
        assert all(
            all(r[k] == seed[k] for k in ("routing", "routing_revision", "revision"))
            for r in checks
        )
        releases = [r for r in records if r["phase"] == "released"]
        assert len(releases) == 100 and all(
            r["actual_rule_controls_destroyed"] and r["observed_objects"] == 378
            for r in releases
        )
        closed = [r for r in records if r["phase"] == "closed_gate"]
        quiet = [r for r in records if r["phase"] == "hidden_quiet"]
        opened = [r for r in records if r["phase"] == "reopened_gate"]
        assert len(closed) == len(quiet) == len(opened) == 100
        assert all(
            q["frames"] == c["frames"]
            and not q["effective_visible"]
            and q["t_ns"] - c["t_ns"] >= 90e6
            for c, q in zip(closed, quiet, strict=True)
        )
        assert all(
            o["loaded_gate_observations"] - c["loaded_gate_observations"] == 1
            for c, o in zip(closed, opened, strict=True)
        )
        assert all(
            opened[i]["loaded_gate_observations"]
            - opened[i - 1]["loaded_gate_observations"]
            == 2
            for i in range(1, 100)
        )
        assert all(r["fd"] == 29 for r in quiet)
        summary = {}
        for phase in PHASES:
            rows = [r for r in records if r["phase"] == phase and r["cycle"] >= 10]
            assert len(rows) == 90
            summary[phase] = {
                key: percentile([r[key] for r in rows])
                for key in ("dispatch_to_ready_ms", "dispatch_to_frame_ms")
            }
            assert summary[phase]["dispatch_to_frame_ms"] <= 100
        summary["close_to_backend_gate_p95_ms"] = percentile(
            [r["dispatch_to_gate_ms"] for r in closed if r["cycle"] >= 10]
        )
        summary["windows"] = {}
        for lo, hi in [(0, 9), (10, 39), (40, 69), (70, 99)]:
            rows = [r for r in opened if lo <= r["cycle"] <= hi]
            summary["windows"][f"{lo}-{hi}"] = {
                key: [min(r[key] for r in rows), max(r[key] for r in rows)]
                for key in (
                    "pss_kib",
                    "private_dirty_kib",
                    "pss_anon_kib",
                    "pss_file_kib",
                    "fd",
                    "threads",
                )
            }
        runs.append(
            {
                "name": run["name"],
                "summary": summary,
                "records": records,
                "backend_trace": run["trace"],
            }
        )
    controls = []
    for campaign in (preflight, model):
        for run in campaign["runs"]:
            controls.append(
                {
                    "name": run["name"],
                    "exit": run["exit"],
                    "terminal": convert(run["records"][-1]),
                    "source_sha256": campaign["source_sha256"],
                }
            )
    output.write_text(
        json.dumps(
            {
                "scope": "P-03 controlled production lifecycle; full Linux/physical/GPU OPEN",
                "cycles": 300,
                "warmup_each": 10,
                "percentile": "nearest rank; 90 samples per action per repetition",
                "source_sha256": raw["source_sha256"],
                "binary_sha256": raw["binary_sha256"],
                "controls": controls,
                "runs": runs,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )
    for run in runs:
        print(run["name"], json.dumps(run["summary"]))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    report(args.base, args.output)


if __name__ == "__main__":
    main()
