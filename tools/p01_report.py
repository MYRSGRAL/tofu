"""Сводка трёх полных P01 по сохранённым terminal/samples, без запуска GUI."""

import json
import statistics
from pathlib import Path

ROOT = Path("/home/myrs/projects/yonik/local/validation/p01-178")


def main() -> None:
    result: dict = {
        "kind": "P01 observer ELF/private VM",
        "stats_gate": "N_A-not-measured",
        "repeats": [],
    }
    for index in range(1, 4):
        root = ROOT / f"repeat-{index}"
        terminal = json.loads((root / "terminal.json").read_text())
        if terminal["state"] != "COMPLETE" or not terminal.get("reaped"):
            raise RuntimeError("Incomplete/reap gate")
        samples = [
            json.loads(x) for x in (root / "samples.jsonl").read_text().splitlines()
        ]
        phases = []
        for phase in terminal["phases"]:
            if phase["duration"] < 900:
                raise RuntimeError("Short P01 phase")
            rows = [x for x in samples if x["phase"] == phase["phase"]]
            groups = {}
            for name in phase["first"]:
                first = phase["first"][name]
                last = phase["last"][name]
                if first["start_ticks"] != last["start_ticks"]:
                    raise RuntimeError("PID reused")
                seconds = (last["cpu_ticks"] - first["cpu_ticks"]) / terminal[
                    "clock_ticks"
                ]
                memory = {}
                for key in ["Pss", "Private_Dirty"]:
                    values = [x["processes"][name]["memory_kib"][key] for x in rows]
                    memory[key] = {
                        "min": min(values),
                        "median": statistics.median(values),
                        "max": max(values),
                    }
                observations = [first, *[x["processes"][name] for x in rows], last]
                stable_threads = all(
                    set(x["threads"]) == set(first["threads"]) for x in observations
                )
                wakeups = None
                if stable_threads and all(
                    v is not None for x in observations for v in x["threads"].values()
                ):
                    deltas = [
                        last["threads"][tid] - first["threads"][tid]
                        for tid in first["threads"]
                    ]
                    if all(delta >= 0 for delta in deltas):
                        wakeups = sum(deltas)
                groups[name] = {
                    "cpu_seconds": seconds,
                    "cpu_percent_one_cpu": seconds / phase["duration"] * 100,
                    "memory_kib": memory,
                    "scheduler_wakeups": wakeups,
                    "wakeup_threads_stable": stable_threads,
                    "wakeup_source": first["wakeup_source"],
                }
            if any(
                state.get("can_stop") or state.get("status") != "Disconnected"
                for state in [phase["start"], phase["end"]]
            ):
                raise RuntimeError("No-connect boundary gate")
            expected_visible = phase["phase"] == "static-open"
            if (
                phase["start"]["visible"] != expected_visible
                or phase["end"]["visible"] != expected_visible
                or phase["start"]["visibility_events"]
                != phase["end"]["visibility_events"]
            ):
                raise RuntimeError("Phase visibility changed")
            frame_delta = phase["end"]["frames"] - phase["start"]["frames"]
            if phase["phase"] == "hidden" and (frame_delta or phase["end"]["visible"]):
                raise RuntimeError("Hidden frame/visibility gate")
            summed_pss = [
                x["processes"]["gui"]["memory_kib"]["Pss"]
                + x["processes"]["tray"]["memory_kib"]["Pss"]
                for x in rows
            ]
            phases.append(
                {
                    "phase": phase["phase"],
                    "duration": phase["duration"],
                    "groups": groups,
                    "frames_delta": frame_delta,
                    "gui_tray_pss_kib": {
                        "min": min(summed_pss),
                        "median": statistics.median(summed_pss),
                        "max": max(summed_pss),
                    },
                    "gui_tray_cpu_percent": groups["gui"]["cpu_percent_one_cpu"]
                    + groups["tray"]["cpu_percent_one_cpu"],
                }
            )
        result["repeats"].append({"repeat": index, "phases": phases})
    result["variance"] = {}
    for phase in ["hidden", "static-open"]:
        phases = [
            next(x for x in r["phases"] if x["phase"] == phase)
            for r in result["repeats"]
        ]
        series = {
            "cpu_percent": [x["gui_tray_cpu_percent"] for x in phases],
            "pss_median_kib": [x["gui_tray_pss_kib"]["median"] for x in phases],
            "frames": [x["frames_delta"] for x in phases],
        }
        for group in ["gui", "tray"]:
            series[group + "_wakeups"] = [
                x["groups"][group]["scheduler_wakeups"] for x in phases
            ]
            series[group + "_private_dirty_median_kib"] = [
                x["groups"][group]["memory_kib"]["Private_Dirty"]["median"]
                for x in phases
            ]
        result["variance"][phase] = {
            name: None
            if any(v is None for v in values)
            else {
                "min": min(values),
                "max": max(values),
                "median": statistics.median(values),
            }
            for name, values in series.items()
        }
    (ROOT / "summary.json").write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
