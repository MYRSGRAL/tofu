"""Включение коротких campaign controls в стандартный Python CI discovery."""

import unittest
from pathlib import Path


def load_tests(
    _loader: unittest.TestLoader,
    _tests: unittest.TestSuite,
    _pattern: str | None,
) -> unittest.TestSuite:
    directory = Path(__file__).resolve().parents[1] / "importcampaign" / "tests"
    return unittest.TestLoader().discover(str(directory))
