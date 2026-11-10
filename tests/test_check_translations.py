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

    def test_tray_pairs(self) -> None:
        source = (ROOT / "crates/yonik-desktop/src/application.cpp").read_text()
        self.assertEqual(checker.check_cpp_source(source), 9)
        with self.assertRaises(checker.CatalogError):
            checker.check_cpp_source(
                source.replace('QStringLiteral("Connect")', 'QStringLiteral("")')
            )

    def test_owned_component(self) -> None:
        path = ROOT / checker.CATALOG.parent / "DiagnosticsExport.qml"
        source = path.read_text()
        self.assertEqual(checker.check_source(source, False), (4, 0))
        with self.assertRaises(checker.CatalogError):
            checker.check_source(source.replace('"Copied"', '""'), False)

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

    def test_other_declarations_are_checked(self) -> None:
        for declaration in ("let", "var"):
            with self.subTest(declaration=declaration):
                source = SOURCE.replace("const", declaration)
                self.assertEqual(checker.check_source(source), (1, 1))
                with self.assertRaises(checker.CatalogError):
                    checker.check_source(SOURCE + source.replace('"Готово"', '""'))

    def test_wrapped_maps_are_rejected(self) -> None:
        for opening, closing in (("(", ")"), ("((", "))")):
            with self.subTest(opening=opening), self.assertRaises(checker.CatalogError):
                checker.check_source(
                    SOURCE + f'const wrapped = {opening}{{ready: ["Да"]}}{closing};'
                )

    def test_single_quoted_keys_are_rejected(self) -> None:
        with self.assertRaises(checker.CatalogError):
            checker.check_source(
                SOURCE + 'const m = {ready: ["Да", "Yes"], \'ready\': ["Нет", "No"]};'
            )


if __name__ == "__main__":
    unittest.main()
