"""Отказ на phantom refs и рассинхронизации принятого snapshot ledger."""

import copy
import json
import subprocess
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

import check_linux_delta_302f8bf as checker


class ЦелостностьDelta(unittest.TestCase):
    def setUp(self) -> None:
        self.folder = Path(__file__).parent
        self.data = json.loads(
            (self.folder / "2026-11-14-linux-criteria-302f8bf.json").read_text()
        )

    def отклонить(self, data: dict[str, Any]) -> None:
        with patch.object(checker.json, "loads", return_value=data):
            with self.assertRaises((AssertionError, subprocess.CalledProcessError)):
                checker.проверить()

    def test_исходный_snapshot_проходит(self) -> None:
        checker.проверить()

    def test_phantom_merge_отклоняется(self) -> None:
        data = copy.deepcopy(self.data)
        data["pull_requests"][0]["merge"] = "0" * 40
        self.отклонить(data)

    def test_существующий_чужой_merge_отклоняется(self) -> None:
        data = copy.deepcopy(self.data)
        pr = next(p for p in data["pull_requests"] if p["pr"] == 185)
        pr["merge"] = data["base"]
        self.отклонить(data)

    def test_merge_in_accepted_пересчитывается(self) -> None:
        data = copy.deepcopy(self.data)
        data["pull_requests"][0]["merge_in_accepted"] = False
        self.отклонить(data)

    def test_phantom_preserved_ref_отклоняется(self) -> None:
        data = copy.deepcopy(self.data)
        data["preserved_pending_refs"][0]["sha"] = "0" * 40
        self.отклонить(data)

    def test_preserved_ancestry_пересчитывается(self) -> None:
        data = copy.deepcopy(self.data)
        data["preserved_pending_refs"][0]["in_accepted"] = True
        self.отклонить(data)

    def test_stale_delta_claim_отклоняется(self) -> None:
        data = copy.deepcopy(self.data)
        data["delta_criteria"][0] = "STALE CLAIM"
        self.отклонить(data)

    def test_изменённый_markdown_отклоняется(self) -> None:
        original = Path.read_text

        def прочитать(path: Path, *args: Any, **kwargs: Any) -> str:
            text = original(path, *args, **kwargs)
            if path.name == "2026-11-14-linux-delta-302f8bf.md":
                return text.replace(
                    self.data["delta_criteria"][0], "| STALE CLAIM |", 1
                )
            return text

        with patch.object(Path, "read_text", прочитать):
            with self.assertRaises(AssertionError):
                checker.проверить()


if __name__ == "__main__":
    unittest.main()
