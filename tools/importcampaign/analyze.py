"""Сводка JSONL с раздельными счётчиками вызовов и записей."""

import argparse
import collections
import json
from pathlib import Path


def analyze(path: Path) -> dict:
    statuses = collections.Counter()
    records = collections.Counter()
    hashes = set()
    families = {}
    total = 0
    with path.open() as stream:
        for line in stream:
            row = json.loads(line)
            total += 1
            statuses[row["status"]] += 1
            hashes.add(row["sha256"])
            family = families.setdefault(
                row["family"], {"calls": 0, "max_seconds": 0, "max_rss_kib": 0}
            )
            family["calls"] += 1
            family["max_seconds"] = max(family["max_seconds"], row["seconds"])
            family["max_rss_kib"] = max(family["max_rss_kib"], row["rss_kib"])
            if row["status"] == "parsed":
                accepted, rejected, reviewed, native = map(int, row["output"].split())
                records.update(
                    accepted=accepted,
                    rejected=rejected,
                    reviewed=reviewed,
                    native_calls=native,
                )
            if "CANARY" in row["output"]:
                raise ValueError(
                    "Синтетический секрет оказался в диагностическом выводе"
                )
    return {
        "calls": total,
        "unique_inputs": len(hashes),
        "statuses": statuses,
        "records": records,
        "families": families,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("jsonl", type=Path)
    args = parser.parse_args()
    print(json.dumps(analyze(args.jsonl), indent=2, ensure_ascii=False))
