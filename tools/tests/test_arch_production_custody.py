"""OCR-проверка readonly отказа не принимает одно предупреждение о формате."""

import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "production_route", Path(__file__).parents[1] / "arch_production_route.py"
)
route = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(route)


class WriteRefusalTest(unittest.TestCase):
    def test_actual_cyrillic_and_ocr_homoglyph(self) -> None:
        for text in (
            "Не удалось сохранить настройки. preferences.json",
            "He удалось сохранить настройки. preferences.json",
        ):
            with self.subTest(text=text):
                self.assertTrue(route.write_refused(text))

    def test_format_warning_is_not_write_failure(self) -> None:
        self.assertFalse(
            route.write_refused(
                "preferences.json неизвестный формат; запись заблокирована"
            )
        )

    def test_success_is_not_write_failure(self) -> None:
        self.assertFalse(route.write_refused("preferences.json: настройки сохранены"))

    def test_unrelated_failure_is_not_enough(self) -> None:
        self.assertFalse(route.write_refused("Не удалось сохранить настройки"))


if __name__ == "__main__":
    unittest.main()
