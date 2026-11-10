"""Проверка реального каталога и повреждений обоих способов перевода."""

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_translations", ROOT / "tools/check_translations.py"
)
assert SPEC is not None and SPEC.loader is not None
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)
SOURCE = 'const messages = {ready: ["Готово", "Ready"]}; tr("Привет", "Hello")'


class TranslationTests(unittest.TestCase):
    def test_current_catalog(self) -> None:
        inline, keys = checker.check_source((ROOT / checker.CATALOG).read_text())
        self.assertGreater(inline, 0)
        self.assertGreater(keys, 0)

    def test_corruptions(self) -> None:
        for old, new in [
            ('"Готово", "Ready"', '"Готово"'),
            ('"Готово", "Ready"', '"Готово", ""'),
            ('"Привет", "Hello"', '"Привет", " "'),
            ('"Привет", "Hello"', '"Привет %1", "Hello %2"'),
            ('"Готово", "Ready"', '"Готово {name}", "Ready"'),
            ('"Готово", "Ready"', '"Готово", "Ready", "Extra"'),
            ("ready:", 'ready: ["Да", "Yes"], ready:'),
            ('tr("Привет", "Hello")', "tr(value, other)"),
        ]:
            with self.subTest(new=new), self.assertRaises(checker.CatalogError):
                checker.check_source(SOURCE.replace(old, new))

    def test_escaped_and_comments(self) -> None:
        checker.check_source(SOURCE + '\n// tr("bad", "")\n/* const x = { */')

    def test_placeholder_multiplicity(self) -> None:
        with self.assertRaises(checker.CatalogError):
            checker.validate_pair("key", "%1 %1", "%1")

    def test_empty_catalog(self) -> None:
        with self.assertRaises(checker.CatalogError):
            checker.check_source("")


if __name__ == "__main__":
    unittest.main()
