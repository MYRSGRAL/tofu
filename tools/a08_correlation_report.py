#!/usr/bin/env python3
"""Проверка scoped A-08 correlation proof; historical evidence не переопределяет."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from tools.a08_report import oracle, require, rows

ROOT = Path(__file__).resolve().parents[1]


def phases(work: Path) -> list[dict]:
    return [
        json.loads(line[4:])
        for line in (work / "gui.log").read_text().splitlines()
        if line.startswith("A08 {")
    ]


def correlation(records: list[dict]) -> None:
    require(
        records and records[-1]["phase"] == "completed", "Нет завершённого GUI proof"
    )
    require(not any(p["phase"] == "FAIL" for p in records), "GUI oracle отверг прогон")
    named = {p["phase"]: p for p in records}
    stale = named["stale_rejected"]
    require(
        stale["expected"] == 0 and stale["rows"] == 0,
        "Stale reply отображён новым Loader",
    )
    require(stale["reply"]["request_id"] > 0, "Нет настоящего delayed reply")
    require(
        stale["new_admissions"] == 0 and stale["new_reads"] == 0,
        "Close/reopen запустил scan",
    )
    newer = named["desktop_after_reopen"]
    require(
        newer["expected"] > stale["reply"]["request_id"],
        "ID повторился после Loader recreation",
    )
    require(
        newer["reply"]["request_id"] == newer["expected"],
        "Desktop reply не коррелирован",
    )
    require(
        all("pid" not in item for item in newer["reply"]["items"]),
        "Desktop показывает process rows",
    )
    for phase in ("query_invalidated", "mode_invalidated"):
        require(
            named[phase]["expected"] == 0 and named[phase]["rows"] == 0,
            "Input не инвалидирует display",
        )
    require(
        named["page_correlated"]["offset"] == 32
        and named["page_correlated"]["rows"] > 0,
        "Нет page correlation",
    )
    older = named["older_reply_rejected"]
    require(
        0 < older["older_reply"] < older["current_expected"],
        "Нет старого A при expected B",
    )
    legacy = named["legacy_shape"]["reply"]
    require(
        set(legacy) == {"items", "total", "offset", "incomplete"},
        "Legacy shape изменена",
    )
    errors = [p for p in records if p["phase"] == "error_preserved"]
    require(
        {p["request_id"] for p in errors} == {0, 7777}, "Оба error paths не проверены"
    )
    require(
        all(p["error"] == "limit" and p["new_admissions"] == 0 for p in errors),
        "Error paths изменены",
    )
    require(
        records[-1]["last_safe_id"] == 9_007_199_254_740_991,
        "Нет safe integer boundary",
    )
    require(
        named["edit_preserved"]["routing"] == records[-1]["routing"],
        "Условия policy изменены",
    )
    require(
        named["edit_preserved"]["revision"] == records[-1]["revision"],
        "Revision изменена",
    )


def report(work: Path) -> dict:
    records = phases(work)
    correlation(records)
    main_sha = hashlib.sha256(
        (ROOT / "crates/yonik-desktop/qml/Main.qml").read_bytes()
    ).hexdigest()
    require(
        "A08 MAIN_SHA256 " + main_sha in (work / "gui.log").read_text(),
        "Original qrc Main mismatch",
    )
    events = rows(work / "host/catalog.jsonl")
    admitted = [e["token"] for e in events if e["event"] == "admitted"]
    released = [e["token"] for e in events if e["event"] == "released"]
    require(
        admitted == list(range(1, len(admitted) + 1)) and len(admitted) == 13,
        "Нет реального producer canary",
    )
    require(released == admitted, "Lifecycle release потерян/повторён")
    require(
        all(
            any(e["event"] == "read" and e["token"] == token for e in events)
            for token in admitted
        ),
        "Пустой producer canary",
    )
    proc = oracle(rows(work / "canary.jsonl"), rows(work / "proc.jsonl"))
    require((work / "marker-control").is_file(), "Нет Exec positive control")
    require(not (work / "EXEC-MUST-NOT-EXIST").exists(), "Exec был запущен")
    named = {p["phase"]: p for p in records}
    require(
        named["known_unsupported"]["diagnostics"]["error"] == "unsupported_selector",
        "Точный selector mapping потерян",
    )
    return {
        "controlled_a08": "passed",
        "physical_linux": "deferred_not_pass",
        "full_g_app_fr20_n5": "not_implied",
        "catalog_incomplete": named["desktop"]["catalog"]["incomplete"],
        "producer_tokens": admitted,
        "stale_reply": named["stale_rejected"],
        "older_reply_after_later_request": named["older_reply_rejected"],
        "legacy_reply_keys": sorted(named["legacy_shape"]["reply"]),
        "last_safe_id": records[-1]["last_safe_id"],
        "proc_syscalls": proc,
        "boundary": "private Xvfb/software, own same-host-UID PID/mount/net namespace; unchanged filename trace",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.write_text(
        json.dumps(report(args.work), ensure_ascii=False, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
