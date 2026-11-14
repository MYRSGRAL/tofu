"""DATA ONLY: чистый join JSON; нет imports драйвера, процессов и сетевых операций."""

import hashlib
import json
import statistics
from pathlib import Path

BASE = Path("/home/myrs/projects/yonik/local/evidence")
HERE = Path(__file__).resolve().parent
OLD = BASE / "p02-short-private-20261114/short-campaign2"
NEW = BASE / "p02-replacement-sb2-reviewed"
INPUTS: dict[str, str] = {}


def digest(path: Path) -> str:
    """Читать потоково, включая ELF."""
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path: Path) -> object:
    """Привязать каждый прочитанный JSON к его байтам."""
    INPUTS[str(path)] = digest(path)
    return json.loads(path.read_text())


def require(value: bool, message: str) -> None:
    """Не выдавать частичный join при нарушении binding."""
    if not value:
        raise ValueError(message)


def spread(values: list[float]) -> dict:
    """Описательный разброс, без доверительных интервалов при n=3."""
    return {"min": min(values), "median": statistics.median(values), "max": max(values)}


def window(path: Path, summary: dict, expected_sha: str | None) -> dict:
    """Пересчитать группу и проверить identity во всех сохранённых точках."""
    rows = read(path / "samples.json")
    if expected_sha:
        require(
            INPUTS[str(path / "samples.json")] == expected_sha, "Изменён старый sample"
        )
    reference = rows[0]["processes"]
    pids = [p["pid"] for p in reference]
    require(len(rows) == 31 and len(set(pids)) == len(pids), "Набор rows/PID")
    require(summary["sampler_outside_group"] not in pids, "Sampler внутри группы")
    require(summary["group_pids"] == pids, "Summary PID mismatch")
    previous = -1.0
    for row in rows:
        require(row["elapsed_s"] > previous, "Немонотонный elapsed")
        previous = row["elapsed_s"]
        require([p["pid"] for p in row["processes"]] == pids, "Изменён PID set")
        for p, ref in zip(row["processes"], reference, strict=True):
            for key in ["starttime", "namespaces", "descendants"]:
                require(p[key] == ref[key], "Identity/group drift")
            require(set(p["descendants"]) <= set(pids), "Child вне группы")
            require(p["wakeups"] is None, "Неожиданные wakeups")
    first, last = rows[0]["processes"], rows[-1]["processes"]
    ticks = sum(
        b["cpu_ticks"] - a["cpu_ticks"] for a, b in zip(first, last, strict=True)
    )
    # Архивные summaries задают SC_CLK_TCK=100; проверяем каждое окно, не читаем host.
    require(abs(ticks / 100 - summary["cpu_s"]) < 1e-12, "CPU tick conversion")
    result = {
        "duration_s": rows[-1]["elapsed_s"],
        "rows": len(rows),
        "cpu_ticks_delta": ticks,
        "cpu_s": ticks / 100,
        "uss_end_kib": None,
        "uss_change_kib": None,
        "wakeups": None,
        "sampler_pid": summary["sampler_outside_group"],
        "group_pids": pids,
        "identities": [{"pid": p["pid"], "starttime": p["starttime"]} for p in first],
        "samples_path": str(path / "samples.json"),
        "samples_sha256": INPUTS[str(path / "samples.json")],
    }
    for raw, name in [("pss_kib", "pss"), ("private_dirty_kib", "private_dirty")]:
        result[f"{name}_start_kib"] = sum(p[raw] for p in first)
        result[f"{name}_end_kib"] = sum(p[raw] for p in last)
        result[f"{name}_change_kib"] = (
            result[f"{name}_end_kib"] - result[f"{name}_start_kib"]
        )
        result[f"{name}_range_kib"] = spread(
            [sum(p[raw] for p in row["processes"]) for row in rows]
        )
        require(
            result[f"{name}_end_kib"] == summary[f"{name}_end_kib"],
            "Memory summary mismatch",
        )
    key = "main_thread_context_switches_proxy_not_wakeups"
    result["main_thread_context_delta_proxy_not_wakeups"] = {
        k: sum(b[key][k] - a[key][k] for a, b in zip(first, last, strict=True))
        for k in ["voluntary_ctxt_switches", "nonvoluntary_ctxt_switches"]
    }
    result["processes"] = [
        {
            "role": "gui_with_integrated_product_tray"
            if len(pids) == 2 and i == 0
            else "engine",
            "pid": b["pid"],
            "cpu_ticks_delta": b["cpu_ticks"] - a["cpu_ticks"],
            "pss_end_kib": b["pss_kib"],
            "private_dirty_end_kib": b["private_dirty_kib"],
            "uss_end_kib": None,
            "fd_start": a["fd"],
            "fd_end": b["fd"],
            "threads_start": a["threads"],
            "threads_end": b["threads"],
        }
        for i, (a, b) in enumerate(zip(first, last, strict=True))
    ]
    require(
        abs(result["duration_s"] - summary["duration_s"]) < 1e-9,
        "Duration summary mismatch",
    )
    require(900 <= result["duration_s"] < 902, "Неполное окно")
    return result


def main() -> None:
    """Воспроизводимый stdout JSON; запись выбирает вызывающий пользователь."""
    frozen = json.loads((HERE / "inputs.json").read_text())
    for name, pin in frozen.items():
        require(digest(Path(name)) == pin, f"Изменён input: {name}")
    validation = read(BASE / "p02-short-final-20261114/validation.json")
    controls = read(BASE / "p02-r2-targeted-20261114/completion.json")
    old_binding = read(OLD / "binding.json")
    terminal = read(BASE / "p02-sb2-single-execute-terminal.json")
    new_result = read(NEW / "result.json")
    require(
        digest(BASE / "p02-short-private-20261114/yonik-desktop")
        == old_binding["binary_sha256"],
        "GUI ELF pin",
    )
    require(
        digest(BASE / "p02-short-private-20261114/observer.so")
        == old_binding["observer_sha256"],
        "Observer pin",
    )
    require(
        digest(BASE / "p02-one-pair-closure-b1b9ce1/manifest.json")
        == "21f90aa0776ba26870c0e691c7479d9a122efbd64e961ae4beac8d781b51033e",
        "Closure pin",
    )
    require(
        terminal["exit_code"] == 0 and not terminal["live_owned_identities"],
        "Replacement terminal",
    )
    require(
        terminal["result_sha256"] == INPUTS[str(NEW / "result.json")],
        "Replacement result SHA",
    )
    require(
        not terminal["pin_errors"]
        and not terminal["failure_json_present"]
        and not terminal["cleanup_fallbacks"],
        "Replacement cleanup",
    )
    indexes = {w["window"]: w for w in validation["windows"]}
    pairs = []
    for core in ["sing-box", "xray"]:
        for repeat in [1, 2, 3]:
            replacement = core == "sing-box" and repeat == 2
            paths = {
                g: NEW / g if replacement else OLD / f"{core}-{repeat}-{g}"
                for g in ["full", "engine"]
            }
            config = paths["full"] / "actual-config.json"
            INPUTS[str(config)] = digest(config)
            engine_config = paths["engine"] / "actual-config.json"
            INPUTS[str(engine_config)] = digest(engine_config)
            require(
                config.read_bytes() == engine_config.read_bytes(),
                "Config bytes mismatch",
            )
            bindings = sorted(paths["full"].glob("engine-binding-*.json"))
            require(len(bindings) == 1, "Не один captured engine")
            binding = read(bindings[0])
            pin = controls["same_pins"][
                "sing-box-1.14.4/sing-box"
                if core == "sing-box"
                else "xray-26.10.31/xray"
            ]
            require(
                binding["config_sha256"] == digest(config)
                and binding["exe_sha256"] == pin,
                "Capture binding",
            )
            windows = {}
            for group, path in paths.items():
                summary = (
                    new_result[group] if replacement else read(path / "summary.json")
                )
                expected = None if replacement else indexes[path.name]["sample_sha256"]
                windows[group] = window(path, summary, expected)
            infrastructure = read(paths["full"] / "infrastructure.json")
            require(
                infrastructure["sampler"] == windows["full"]["sampler_pid"],
                "Infrastructure sampler",
            )
            require(
                infrastructure["product_gui"] == windows["full"]["group_pids"][0],
                "GUI binding",
            )
            require(binding["pid"] == windows["full"]["group_pids"][1], "Captured PID")
            require(
                all(
                    v not in windows["full"]["group_pids"]
                    for k, v in infrastructure.items()
                    if k != "product_gui"
                ),
                "Infrastructure inside group",
            )
            delta = {
                k: windows["full"][k] - windows["engine"][k]
                for k in ["cpu_s", "pss_end_kib", "private_dirty_end_kib"]
            }
            delta["cpu_s"] = (
                windows["full"]["cpu_ticks_delta"]
                - windows["engine"]["cpu_ticks_delta"]
            ) / 100
            delta["uss_end_kib"] = None
            delta["main_thread_context_delta_proxy_not_wakeups"] = {
                k: windows["full"]["main_thread_context_delta_proxy_not_wakeups"][k]
                - windows["engine"]["main_thread_context_delta_proxy_not_wakeups"][k]
                for k in ["voluntary_ctxt_switches", "nonvoluntary_ctxt_switches"]
            }
            pairs.append(
                {
                    "core": core,
                    "repeat": repeat,
                    "origin": "replacement"
                    if replacement
                    else "bounded_observational_reuse",
                    "measurement_source_commit": "b1b9ce10525082039ecbdf87a6abbf33fc11449f"
                    if replacement
                    else validation["measurement_source"],
                    "driver_sha256": "508f837200ffa07aaaca6e88cc296c41dd789bf6dfbd9aa054d6bd8c43536f23"
                    if replacement
                    else "4e51d4676132da15e957479c84b404c3e3be0aa9adb31a766d796cfb0ff47ce7",
                    "driver_source_commit": "51fd15a3a94c1e5e2ca400e1dcd7ccf26438715e"
                    if replacement
                    else validation["measurement_source"],
                    "runtime_held_pidfd_guards": replacement,
                    "config_sha256": digest(config),
                    "config_bytes_identical": True,
                    "core_elf_sha256": pin,
                    "gui_elf_sha256": old_binding["binary_sha256"],
                    "observer_sha256": old_binding["observer_sha256"],
                    "infrastructure_excluded": infrastructure,
                    "windows": windows,
                    "full_minus_engine": delta,
                    "cleanup_evidence": str(
                        BASE / "p02-sb2-single-execute-terminal.json"
                    )
                    if replacement
                    else str(BASE / "p02-short-final-20261114/validation.json"),
                }
            )
    aggregate = {}
    for core in ["sing-box", "xray"]:
        subset = [p for p in pairs if p["core"] == core]
        aggregate[core] = {
            g: {
                k: spread([p["windows"][g][k] for p in subset])
                for k in ["cpu_s", "pss_end_kib", "private_dirty_end_kib"]
            }
            for g in ["full", "engine"]
        }
        aggregate[core]["paired_full_minus_engine"] = {
            k: spread([p["full_minus_engine"][k] for p in subset])
            for k in ["cpu_s", "pss_end_kib", "private_dirty_end_kib"]
        }
    excluded = {
        "core": "sing-box",
        "repeat": 2,
        "origin": str(OLD),
        "excluded_groups": ["full", "engine"],
        "reason": "Полная старая pair исключена из paired comparison: targeted-control overlap в full; raw сохранён.",
        "overlap_seconds": controls["finished_unix"] - controls["started_unix"],
        "offsets_approx_s": [55.04, 203.37],
        "offset_method": "mtime минус elapsed, не per-row wallclock",
        "samples_sha256": {
            g: indexes[f"sing-box-2-{g}"]["sample_sha256"] for g in ["full", "engine"]
        },
    }
    for name, value in INPUTS.items():
        require(frozen.get(name) == value, f"Не зафиксирован input: {name}")
    output = {
        "classification": "SIX_PAIR_DATA_JOIN_REVIEW_REQUIRED_BOUNDED_OBSERVATIONAL",
        "pair_count": 6,
        "window_count": 12,
        "clock_ticks_per_second": 100,
        "order": "full→engine; fixed order bias",
        "pairs": pairs,
        "aggregate": aggregate,
        "excluded": excluded,
        "wakeups": None,
        "uss": None,
        "uss_reason": "Private_Clean не сохранён; Private_Dirty не равен USS.",
        "controls": controls,
        "old_measurement_binding": old_binding,
        "validator_source_commit": validation["validator_source"],
        "replacement_terminal": terminal,
        "qualifications": [
            "R-A: старые пять пар — только bounded observational snapshots; новые guards не применялись ретроактивно.",
            "Нет process-spawn tracing между samples; отсутствие краткоживущих children не доказано.",
            "GUI включает integrated product tray; private SNI fixture исключён, отдельной PSS product tray нет.",
            "Private_Dirty — нижняя часть USS; точный USS неизвестен. Context switches — только main-thread proxy, не wakeups.",
            "Dev/unoptimized GUI, LocalSocks/no-app-rules/offscreen/software/private SNI; старые AC/powersave conditions не доказывают условия новой пары.",
            "Нет процента CPU overhead при near-zero baseline; нет energy/GPU/physical/8h/final Linux P02 PASS.",
            "Нет зарегистрированного targeted-control overlap в retained пяти; иная host-нагрузка не исключена.",
        ],
        "inputs_manifest_sha256": digest(HERE / "inputs.json"),
    }
    print(json.dumps(output, ensure_ascii=False, sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
