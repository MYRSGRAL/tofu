"""Negative controls проверяют ограничения supervisor, а не свойства parser."""

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "campaign", Path(__file__).parents[1] / "campaign.py"
)
assert SPEC is not None and SPEC.loader is not None
campaign = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(campaign)


class Ограничения(unittest.TestCase):
    def test_timeout(self) -> None:
        result = campaign.probe(
            [sys.executable, "-c", "while True: pass"], b"", wall=0.1
        )
        self.assertEqual(result["status"], "timeout")
        self.assertLess(result["seconds"], 1)

    def test_memory(self) -> None:
        result = campaign.probe([sys.executable, "-c", "bytearray(512*1024*1024)"], b"")
        self.assertEqual(result["status"], "memory_failure")
        self.assertLess(result["rss_kib"], 256 * 1024)

    def test_слово_memory_не_является_allocator_failure(self) -> None:
        result = campaign.probe(
            [
                sys.executable,
                "-c",
                "import sys; sys.stderr.write('memory unrelated\\n'); sys.exit(1)",
            ],
            b"",
        )
        self.assertEqual(result["status"], "crash")

    def test_summary_разделяет_записи_и_вызовы(self) -> None:
        observations = [
            dict(status="exit", code=0, seconds=0.001, rss_kib=1024, output="2 3 1 0"),
            dict(
                status="exit",
                code=0,
                seconds=0.001,
                rss_kib=1024,
                output="reject TooLarge",
            ),
        ]
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "run"
            with (
                patch.object(
                    campaign,
                    "cases",
                    return_value=[("good", b"a", "any"), ("bad", b"b", "any")],
                ),
                patch.object(campaign, "probe", side_effect=observations),
            ):
                campaign.run(Path(sys.executable), destination, 0, 1, None)
            counts = json.loads((destination / "summary.json").read_text())["counts"]
        self.assertEqual(counts["accepted_records"], 2)
        self.assertEqual(counts["rejected_records"], 3)
        self.assertEqual(counts["reviewed_records"], 1)
        self.assertEqual(counts["parsed_calls"], 1)
        self.assertEqual(counts["rejected_calls"], 1)
        self.assertNotIn("rejected", counts)

    def test_output(self) -> None:
        result = campaign.probe(
            [sys.executable, "-c", "import os\nwhile True: os.write(1,b'x'*4096)"], b""
        )
        self.assertEqual(result["status"], "output_limit")

    def test_cpu(self) -> None:
        result = campaign.probe([sys.executable, "-c", "while True: pass"], b"", wall=4)
        self.assertEqual(result["status"], "crash")
        self.assertEqual(result["code"], -9)
        self.assertLess(result["seconds"], 4)

    def test_descendant(self) -> None:
        script = "import os,time\nif os.fork() == 0: time.sleep(30)"
        result = campaign.probe([sys.executable, "-c", script], b"", wall=0.1)
        self.assertEqual(result["status"], "timeout")
        self.assertLess(result["seconds"], 1)

    def test_reap(self) -> None:
        result = campaign.probe([sys.executable, "-c", "pass"], b"x" * 100000)
        self.assertEqual(result["code"], 0)


if __name__ == "__main__":
    unittest.main()
