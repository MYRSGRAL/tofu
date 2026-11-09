"""Границы безопасного JSON-lines reader native наблюдателя."""

from __future__ import annotations

import json
import os
import sys
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from network_watch_smoke import MAX_LINE, MAX_RECORDS, Observer  # noqa: E402


class ReaderTests(unittest.TestCase):
    def read(self, payload: bytes, *, full: bool = False) -> Observer:
        reader, writer = os.pipe()
        observer = Observer.__new__(Observer)
        observer.fd = reader
        observer.buffer = b""
        observer.records = [self.record()] * MAX_RECORDS if full else []
        observer.eof = False
        try:
            os.write(writer, payload)
            os.close(writer)
            writer = -1
            observer.read_until(time.monotonic() + 1)
            return observer
        finally:
            os.close(reader)
            if writer != -1:
                os.close(writer)

    @staticmethod
    def record() -> dict[str, str | bool]:
        return {
            "epoch": "a" * 32 + ":1",
            "routes": True,
            "resolver": False,
            "manager": False,
            "exhausted": False,
            "closed": False,
        }

    def test_valid_record_and_eof(self) -> None:
        record = self.record()
        observer = self.read(json.dumps(record).encode() + b"\n")
        self.assertEqual(observer.records, [record])
        self.assertTrue(observer.eof)

    def test_payload_values_and_wrong_types_are_rejected(self) -> None:
        for record in (
            {**self.record(), "address": "unexpected"},
            {**self.record(), "routes": 1},
            {**self.record(), "epoch": "unexpected"},
        ):
            with self.subTest(record=record), self.assertRaises(RuntimeError):
                self.read(json.dumps(record).encode() + b"\n")

    def test_line_and_record_limits(self) -> None:
        for payload in (b"x" * (MAX_LINE + 1), b"x" * (MAX_LINE + 1) + b"\n"):
            with self.subTest(size=len(payload)), self.assertRaises(RuntimeError):
                self.read(payload)
        with self.assertRaises(RuntimeError):
            self.read(json.dumps(self.record()).encode() + b"\n", full=True)

    def test_partial_record_at_eof_is_rejected(self) -> None:
        with self.assertRaises(RuntimeError):
            self.read(b'{"epoch":')


if __name__ == "__main__":
    unittest.main()
