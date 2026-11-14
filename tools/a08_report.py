#!/usr/bin/env python3
"""Проверка реального A-08 evidence; physical/native остаются вне приёмки."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def require(value: bool, reason: str) -> None:
    if not value:
        raise ValueError(reason)


def oracle(canary: list[dict], actual: list[dict]) -> dict[str, int]:
    require(
        {r["class"] for r in canary} >= {"cmdline", "environ"},
        "Нет чувствительности IO oracle",
    )
    counts = Counter(r["class"] for r in actual)
    require(counts["decode_error"] == 0, "IO oracle потерял декодирование syscall")
    require(
        not (counts["cmdline"] or counts["environ"]), "Обнаружен запрещённый proc input"
    )
    require(counts["stat"] > 0 and counts["exe"] > 0, "Нет actual proc read canary")
    return dict(counts)


def rows(path: Path) -> list[dict]:
    return [json.loads(s) for s in path.read_text().splitlines()]


def report(work: Path) -> dict:
    log = (work / "gui.log").read_text()
    expected = hashlib.sha256(
        (ROOT / "crates/yonik-desktop/qml/Main.qml").read_bytes()
    ).hexdigest()
    require("A08 MAIN_SHA256 " + expected in log, "Original Main qrc mismatch")
    phases = [json.loads(s[4:]) for s in log.splitlines() if s.startswith("A08 {")]
    require(phases and phases[-1]["phase"] == "completed", "GUI campaign не завершён")
    require(not any(p["phase"] == "FAIL" for p in phases), "GUI oracle отверг прогон")
    named = {p["phase"]: p for p in phases}
    require(
        named["known_unsupported"]["diagnostics"]["error"] == "unsupported_selector",
        "Нет exact UI mapping",
    )
    policy = named["edit_preserved"]["routing"]
    rule = policy["rules"][0]
    require(
        rule["platform_sources"]
        == [{"kind": "android_package", "package": "org.example.App"}],
        "Selector потерян при edit",
    )
    require(rule["domains"] == ["example.com"], "AND condition потеряно при edit")
    require(
        policy == named["completed"]["routing"], "Policy изменилась при catalog actions"
    )
    require(
        named["edit_preserved"]["revision"] == named["completed"]["revision"],
        "Revision изменилась при catalog actions",
    )
    hidden = named["hidden"]
    before = [json.loads(s) for s in hidden["before"].splitlines()]
    after = [json.loads(s) for s in hidden["after"].splitlines()]
    a = Counter(r["event"] for r in before)
    b = Counter(r["event"] for r in after)
    require(
        a["admitted"] == b["admitted"] and a["read"] == b["read"],
        "Новый scan в hidden phase",
    )
    require(b["released"] == a["released"] + 1, "In-flight release не наблюдался")
    events = rows(work / "host/catalog.jsonl")
    admitted = [r["token"] for r in events if r["event"] == "admitted"]
    released = [r["token"] for r in events if r["event"] == "released"]
    require(
        admitted == [1, 2, 3, 4] and released == admitted,
        "Потеря/повтор producer lifecycle",
    )
    require(
        sum(r["event"] == "read" and r["token"] == 4 for r in events) > 0,
        "Нет reopen visible canary",
    )
    require(
        (work / "marker-control").is_file(), "Exec marker positive control отсутствует"
    )
    require(not (work / "EXEC-MUST-NOT-EXIST").exists(), "Desktop Exec был выполнен")
    proc = oracle(rows(work / "canary.jsonl"), rows(work / "proc.jsonl"))
    return {
        "controlled_checks": "passed",
        "a08_complete": False,
        "boundary": "private Xvfb/software, user/PID/mount/net namespace; own host UID only",
        "catalog_incomplete": named["desktop"]["catalog"]["incomplete"],
        "hidden_new_admissions": b["admitted"] - a["admitted"],
        "hidden_new_reads": b["read"] - a["read"],
        "hidden_finishing_release": b["released"] - a["released"],
        "producer_tokens": admitted,
        "proc_syscalls": proc,
        "remaining": "Reopen shows late process catalog while new checkbox defaults to desktop; no session/provenance rejection. Physical Linux deferred.",
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
