"""Счётчики записей не смешиваются с фатальными отказами вызовов."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "analyze", Path(__file__).parents[1] / "analyze.py"
)
assert SPEC is not None and SPEC.loader is not None
analyze = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(analyze)


class Сводка(unittest.TestCase):
    def test_записи_и_вызовы(self) -> None:
        rows = []
        for status, output in (("parsed", "2 3 1 0"), ("rejected", "reject TooLarge")):
            rows.append(
                dict(
                    status=status,
                    output=output,
                    sha256=status,
                    family="probe",
                    seconds=0.01,
                    rss_kib=1024,
                )
            )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "log.jsonl"
            path.write_text("\n".join(json.dumps(row) for row in rows))
            result = analyze.analyze(path)
        self.assertEqual(result["records"]["accepted"], 2)
        self.assertEqual(result["records"]["rejected"], 3)
        self.assertEqual(result["records"]["reviewed"], 1)
        self.assertEqual(result["statuses"]["rejected"], 1)
        self.assertEqual(result["unique_inputs"], 2)


if __name__ == "__main__":
    unittest.main()
