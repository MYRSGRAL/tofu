"""Проверка gate сводки на синтетических JSON, не GUI acceptance."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools import p01_report


class ReportGateTests(unittest.TestCase):
    def fixture(self, root: Path, duration: int = 900, frames: int = 0) -> None:
        for index in range(1, 4):
            output = root / f"repeat-{index}"
            output.mkdir()
            first = {
                "start_ticks": 1,
                "cpu_ticks": 100,
                "scheduler_wakeups": 100,
                "threads": {"1:1": 100},
            }
            last = {
                "start_ticks": 1,
                "cpu_ticks": 110,
                "scheduler_wakeups": 110,
                "threads": {"1:1": 110},
            }
            for value in [first, last]:
                value.update(
                    memory_kib={"Pss": 2048, "Private_Dirty": 1024},
                    wakeup_source="unit fixture",
                )
            phases = [
                {
                    "phase": name,
                    "duration": duration,
                    "first": {"gui": first, "tray": first},
                    "last": {"gui": last, "tray": last},
                    "start": {
                        "frames": 0,
                        "visible": name == "static-open",
                        "visibility_events": 0,
                        "status": "Disconnected",
                        "can_stop": False,
                    },
                    "end": {
                        "frames": frames,
                        "visible": name == "static-open",
                        "visibility_events": 0,
                        "status": "Disconnected",
                        "can_stop": False,
                    },
                }
                for name in ["hidden", "static-open"]
            ]
            (output / "terminal.json").write_text(
                json.dumps(
                    {
                        "state": "COMPLETE",
                        "reaped": True,
                        "clock_ticks": 100,
                        "phases": phases,
                    }
                )
            )
            (output / "samples.jsonl").write_text(
                "\n".join(
                    json.dumps({"phase": x["phase"], "processes": x["last"]})
                    for x in phases
                )
            )

    def test_full_repeats_produce_cpu_memory_wakeup_variance(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            with patch.object(p01_report, "ROOT", root):
                p01_report.main()
            result = json.loads((root / "summary.json").read_text())
            self.assertEqual(result["stats_gate"], "N_A-not-measured")
            self.assertEqual(result["variance"]["hidden"]["gui_wakeups"]["median"], 10)
            self.assertEqual(
                result["variance"]["hidden"]["pss_median_kib"]["median"], 4096
            )
            self.assertAlmostEqual(
                result["variance"]["hidden"]["cpu_percent"]["median"], 0.2 / 900 * 100
            )

    def test_thread_exit_does_not_become_negative_or_zero_wakeups(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            for output in root.glob("repeat-*"):
                path = output / "terminal.json"
                data = json.loads(path.read_text())
                for phase in data["phases"]:
                    phase["first"]["gui"]["threads"] = {"1:1": 300, "2:1": 200}
                path.write_text(json.dumps(data))
            with patch.object(p01_report, "ROOT", root):
                p01_report.main()
            result = json.loads((root / "summary.json").read_text())
            group = result["repeats"][0]["phases"][0]["groups"]["gui"]
            self.assertIsNone(group["scheduler_wakeups"])
            self.assertFalse(group["wakeup_threads_stable"])

    def test_visibility_transition_inside_phase_cannot_pass(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            path = root / "repeat-1" / "terminal.json"
            data = json.loads(path.read_text())
            data["phases"][1]["end"]["visibility_events"] = 2
            path.write_text(json.dumps(data))
            with (
                patch.object(p01_report, "ROOT", root),
                self.assertRaises(RuntimeError),
            ):
                p01_report.main()

    def test_short_phase_and_hidden_frames_cannot_produce_summary(self) -> None:
        for duration, frames in [(899, 0), (900, 1)]:
            with (
                self.subTest(duration=duration, frames=frames),
                tempfile.TemporaryDirectory() as temporary,
            ):
                root = Path(temporary)
                self.fixture(root, duration, frames)
                with (
                    patch.object(p01_report, "ROOT", root),
                    self.assertRaises(RuntimeError),
                ):
                    p01_report.main()
                self.assertFalse((root / "summary.json").exists())


if __name__ == "__main__":
    unittest.main()
