"""Отрицательные controls сравнения синтетических datasets, без GUI."""

import unittest

from test_ui_benchmark import bench


class ComparisonControls(unittest.TestCase):
    def test_unmatched_dataset_is_rejected(self) -> None:
        report = bench.Oracle(10000).expected("frame") | dict(
            toolkit="fake", created_total=200, alive_rows=None, visible_rows=None
        )
        with self.assertRaises(bench.BenchmarkError):
            bench.Oracle(1000).validate(report, "frame")

    def test_invalid_page_is_rejected_even_with_valid_flag(self) -> None:
        oracle = bench.Oracle(1000)
        report = oracle.expected("frame") | dict(
            toolkit="fake",
            created_total=200,
            alive_rows=None,
            visible_rows=None,
            valid=True,
            page_hash="invalid",
        )
        with self.assertRaises(bench.BenchmarkError):
            oracle.validate(report, "frame")
