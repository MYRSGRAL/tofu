"""Подключает изолированную матрицу notices к штатному CI discovery."""

import importlib.util
import unittest
from pathlib import Path


def load_tests(
    loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None
) -> unittest.TestSuite:
    path = (
        Path(__file__).resolve().parents[2] / "tests/tools/test_check_linux_notices.py"
    )
    spec = importlib.util.spec_from_file_location("linux_notices_matrix", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("Не удалось загрузить notices tests")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return loader.loadTestsFromModule(module)
