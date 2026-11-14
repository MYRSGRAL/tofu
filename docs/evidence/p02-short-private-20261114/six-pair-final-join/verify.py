"""Проверки DATA ONLY: детерминизм, независимый пересчёт и отрицательные cases."""

import copy
import hashlib
import json
import subprocess
import tempfile
from decimal import Decimal
from pathlib import Path

import aggregate

HERE = Path(__file__).resolve().parent


def sha(path: Path) -> str:
    """Потоковый hash входа или результата."""
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check(value: bool, message: str) -> None:
    """Падать при первом расхождении evidence."""
    if not value:
        raise ValueError(message)


def main() -> None:
    """Не импортировать/запускать measurement driver; subprocess только pure scripts."""
    d = json.loads((HERE / "comparison.json").read_text())
    check(len(d["pairs"]) == 6, "Кардинальность")
    check(
        sum(p["origin"] == "replacement" for p in d["pairs"]) == 1, "Один replacement"
    )
    check(len({(p["core"], p["repeat"]) for p in d["pairs"]}) == 6, "Уникальность пар")
    check(
        d["excluded"]["excluded_groups"] == ["full", "engine"],
        "Исключена вся noisy pair",
    )
    checked = 0
    for pair in d["pairs"]:
        check(
            not (
                pair["core"] == "sing-box"
                and pair["repeat"] == 2
                and pair["origin"] != "replacement"
            ),
            "Noisy pair внутри join",
        )
        for group in ["full", "engine"]:
            w = pair["windows"][group]
            rows = json.loads(Path(w["samples_path"]).read_text())
            a, b = rows[0]["processes"], rows[-1]["processes"]
            expected_cpu = Decimal(
                sum(p["cpu_ticks"] for p in b) - sum(p["cpu_ticks"] for p in a)
            ) / Decimal(100)
            check(Decimal(str(w["cpu_s"])) == expected_cpu, "Независимый CPU")
            check(w["pss_end_kib"] == sum(p["pss_kib"] for p in b), "Независимый PSS")
            check(
                w["private_dirty_end_kib"] == sum(p["private_dirty_kib"] for p in b),
                "Независимый Private_Dirty",
            )
            check(
                w["uss_end_kib"] is None and w["wakeups"] is None,
                "Unknown не превращён в ноль",
            )
            for key in ["voluntary_ctxt_switches", "nonvoluntary_ctxt_switches"]:
                ctx = "main_thread_context_switches_proxy_not_wakeups"
                expected = sum(p[ctx][key] for p in b) - sum(p[ctx][key] for p in a)
                check(
                    w["main_thread_context_delta_proxy_not_wakeups"][key] == expected,
                    "Независимый context delta",
                )
            checked += 1
        expected_delta = Decimal(str(pair["windows"]["full"]["cpu_s"])) - Decimal(
            str(pair["windows"]["engine"]["cpu_s"])
        )
        check(
            Decimal(str(pair["full_minus_engine"]["cpu_s"])) == expected_delta,
            "Paired CPU exact",
        )
    for script, artifact in [
        ("aggregate.py", "comparison.json"),
        ("report.py", "report.md"),
    ]:
        regenerated = subprocess.run(
            ["/usr/bin/python3", "-B", str(HERE / script)],
            check=True,
            capture_output=True,
        ).stdout
        check(regenerated == (HERE / artifact).read_bytes(), f"Детерминизм {artifact}")
    fixture = d["pairs"][0]["windows"]["full"]
    original = json.loads(Path(fixture["samples_path"]).read_text())
    summary = json.loads(
        (Path(fixture["samples_path"]).parent / "summary.json").read_text()
    )
    negative = []
    for case in [
        "identity_drift",
        "unknown_child",
        "sampler_in_group",
        "truncated_window",
        "sample_hash_mismatch",
    ]:
        rows, s = copy.deepcopy(original), copy.deepcopy(summary)
        if case == "identity_drift":
            rows[-1]["processes"][0]["starttime"] = "invalid"
        elif case == "unknown_child":
            rows[-1]["processes"][0]["descendants"].append(999999999)
        elif case == "sampler_in_group":
            s["sampler_outside_group"] = s["group_pids"][0]
        elif case == "truncated_window":
            rows.pop()
        with tempfile.TemporaryDirectory(prefix="data-check-", dir=HERE) as temp:
            path = Path(temp)
            (path / "samples.json").write_text(json.dumps(rows))
            try:
                aggregate.window(
                    path, s, "0" * 64 if case == "sample_hash_mismatch" else None
                )
            except ValueError:
                negative.append(case)
            else:
                raise ValueError(f"Не отклонён {case}")
    inputs = json.loads((HERE / "inputs.json").read_text())
    for name, expected in inputs.items():
        check(sha(Path(name)) == expected, f"Input изменён после join: {name}")
    result = {
        "status": "DATA_CHECKS_PASSED_REVIEW_REQUIRED",
        "independent_windows_recomputed": checked,
        "deterministic_artifacts": ["comparison.json", "report.md"],
        "negative_cases_rejected": negative,
        "unchanged_input_count": len(inputs),
        "inputs_sha256": sha(HERE / "inputs.json"),
        "comparison_sha256": sha(HERE / "comparison.json"),
        "report_sha256": sha(HERE / "report.md"),
        "runtime_measurements_started": 0,
        "runtime_drivers_imported": 0,
        "ruff": "Не найден в PATH; зависимости не устанавливались.",
    }
    print(json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
