"""Невалидный timing/lifecycle не должен считаться native контрпримером."""

from __future__ import annotations

import contextlib
import sys
import tempfile
import unittest
from collections.abc import Iterator
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import fakeip_epoch_gap as probe  # noqa: E402


def observed_run(elapsed: float, alive: bool = True) -> dict:
    @contextlib.contextmanager
    def process(*_args: object, **_kwargs: object) -> Iterator[SimpleNamespace]:
        child = SimpleNamespace(poll=lambda: None if alive else 1, returncode=None)
        try:
            yield child
        finally:
            child.returncode = 0 if alive else 1

    def dns(*_args: object, **kwargs: object) -> list[str]:
        capture = kwargs["ttl_capture"]
        assert isinstance(capture, list)
        capture.append(1)
        return ["198.18.0.2"]

    with (
        tempfile.TemporaryDirectory(prefix="yonik-dns-observation-") as directory,
        patch.object(probe, "markers", return_value=contextlib.nullcontext([])),
        patch.object(probe, "engine_process", side_effect=process),
        patch.object(probe, "wait_listener"),
        patch.object(probe, "dns_query", side_effect=dns),
        patch.object(probe, "connect", side_effect=[None, "M00", "M01", None, "M01"]),
        patch.object(probe.time, "monotonic", side_effect=[0.0, elapsed, elapsed]),
    ):
        return probe.run_engine(
            Path("unused-synthetic-binary"), "xray", Path(directory)
        )


class NativeObservationTests(unittest.TestCase):
    def test_live_exact_markers_before_ttl_form_counterexample(self) -> None:
        result = observed_run(0.1)
        self.assertTrue(result["same_session_stale_alias"]["within_original_ttl"])
        self.assertEqual(result["same_session_stale_alias"]["stale_ip_marker"], "M01")
        self.assertFalse(result["gate_supported"])
        self.assertTrue(result["engines_reaped"])

    def test_expired_original_ttl_is_not_a_successful_reproduction(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "TTL"):
            observed_run(2.0)

    def test_unexpected_engine_exit_invalidates_observation(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "завершилось"):
            observed_run(0.1, alive=False)


if __name__ == "__main__":
    unittest.main()
