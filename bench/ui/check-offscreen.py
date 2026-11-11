#!/usr/bin/env python3
"""Проверка JSONL без desktop display; не benchmark и не замер энергии."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.artifacts = args.artifacts.resolve()
    manifest = json.loads((args.artifacts / "manifest.json").read_text())
    for toolkit in ("qt", "gtk", "slint"):
        name = f"yonik-bench-{toolkit}"
        digest = hashlib.sha256((args.artifacts / name).read_bytes()).hexdigest()
        if digest != manifest["binary"][name]["sha256"]:
            parser.error(f"SHA-256 не совпадает: {name}")
    args.output.mkdir(parents=True, exist_ok=False)
    spec = importlib.util.spec_from_file_location(
        "ui_benchmark", Path(__file__).resolve().parents[2] / "tools/ui_benchmark.py"
    )
    assert spec is not None and spec.loader is not None
    driver = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(driver)
    # Исключает соединение с заблокированным desktop даже при ошибке backend.
    os.environ.pop("DISPLAY", None)
    os.environ["WAYLAND_DISPLAY"] = "/nonexistent/yonik-offscreen-display"
    os.environ["GTK_A11Y"] = "none"
    os.environ["QT_QPA_PLATFORM"] = "offscreen"
    os.environ["QT_QUICK_BACKEND"] = "software"
    os.environ["GDK_BACKEND"] = "headless"
    cancellation = driver.Cancellation()
    try:
        results = []
        for toolkit in ("qt", "gtk", "slint"):
            for rows in (1000, 10000):
                record = dict(toolkit=toolkit, rows=rows, valid=False, reports=[])
                child = None
                cancellation.protected = True
                cancellation.check()
                try:
                    child = driver.Child(
                        [
                            str(args.artifacts / f"yonik-bench-{toolkit}"),
                            "--rows",
                            str(rows),
                        ],
                        args.output / f"{toolkit}-{rows}.stderr.log",
                    )
                    cancellation.protected = False
                    cancellation.check()
                    oracle = driver.Oracle(rows)
                    report, _ = child.ack(10)
                    oracle.validate(report, "frame")
                    record["reports"].append(report)
                    for command in (
                        driver.query(),
                        driver.query("server-0.example.invalid"),
                        dict(op="select", id=1),
                        dict(op="favorite", id=1, value=True),
                        driver.query(favorites_only=True),
                        driver.query(),
                        driver.query(sort="name"),
                        dict(op="scroll", index=199),
                        dict(op="hide"),
                        dict(op="show"),
                    ):
                        event = oracle.apply(command)
                        child.send(command, 10)
                        report, _ = child.ack(10)
                        oracle.validate(report, event)
                        if report["toolkit"] != record["reports"][0]["toolkit"]:
                            raise driver.BenchmarkError(
                                "Toolkit изменился внутри запуска"
                            )
                        record["reports"].append(report)
                    child.send(dict(op="quit"), 10)
                    child.finish(10)
                    record["valid"] = True
                except (driver.BenchmarkError, OSError) as error:
                    record["error"] = str(error)
                finally:
                    cancellation.protected = True
                    if child is not None:
                        child.close()
                results.append(record)
                (args.output / "protocol.json").write_text(
                    json.dumps(results, ensure_ascii=False, indent=2) + "\n"
                )
        cancellation.check()
        return 0 if all(record["valid"] for record in results) else 1

    except KeyboardInterrupt:
        return 130
    finally:
        cancellation.protected = True
        cancellation.restore()


if __name__ == "__main__":
    raise SystemExit(main())
