"""Отрицательные проверки oracle: DNS witness, timeout и TTL budget."""

from __future__ import annotations

import sys
import unittest
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import dns_pinned_controls as controls


class Controls(unittest.TestCase):
    def test_dead_engine_during_wait_is_fatal(self) -> None:
        process = MagicMock()
        process.poll.return_value = 1
        with self.assertRaises(RuntimeError):
            controls.wait_ttl(600, controls.time.monotonic(), process)

    def test_unbounded_ttl_is_fatal(self) -> None:
        for ttl in [0, -1, 601]:
            with self.subTest(ttl=ttl), self.assertRaises(RuntimeError):
                controls.wait_ttl(ttl, controls.time.monotonic(), MagicMock())

    def test_timeout_never_becomes_an_exclusion_pass(self) -> None:
        @contextmanager
        def process(*args: object) -> Iterator[MagicMock]:
            yield MagicMock()

        with (
            patch.object(controls, "engine_process", process),
            patch.object(controls, "DnsFixture"),
            patch.object(controls, "wait_listener"),
            patch.object(controls, "dns_query", side_effect=TimeoutError),
        ):
            with self.assertRaises(TimeoutError):
                controls.exclusions(Path("/unused"), "xray", False, Path("/unused"))

    def test_real_answers_without_actual_witness_are_rejected(self) -> None:
        @contextmanager
        def process(*args: object) -> Iterator[MagicMock]:
            yield MagicMock()

        answers = [["192.0.2.10"]] * 3 + [["198.18.0.1"]] * 3
        upstream = MagicMock()
        upstream.records = []
        context = MagicMock()
        context.__enter__.return_value = upstream
        with (
            patch.object(controls, "engine_process", process),
            patch.object(controls, "DnsFixture", return_value=context),
            patch.object(controls, "wait_listener"),
            patch.object(controls, "dns_query", side_effect=answers),
        ):
            with self.assertRaisesRegex(RuntimeError, "Upstream witness"):
                controls.exclusions(Path("/unused"), "xray", False, Path("/unused"))


if __name__ == "__main__":
    unittest.main()
