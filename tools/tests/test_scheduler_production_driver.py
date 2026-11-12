"""Observer различает Bridge invalid_input и scheduler companion."""

from __future__ import annotations

import unittest

from check_scheduler_production_ui import production_driver


class ProductionDriver(unittest.TestCase):
    def test_interval_requires_companion_in_both_languages(self) -> None:
        for locale in ("ru", "en"):
            with self.subTest(locale=locale):
                driver = production_driver("interval", locale)
                self.assertIn('c.reason==="scheduler_interval_invalid"', driver)
                self.assertIn('c.stage==="scheduler_validation"', driver)
                self.assertIn(f'const locale="{locale}"', driver)
                self.assertIn('click("copyDiagnosticsButton")', driver)
                self.assertIn('console.log("UI CLOSED "', driver)
                self.assertNotIn("snapshot_json=", driver)

    def test_supported_control_requires_empty_contexts(self) -> None:
        driver = production_driver("interval", "ru", supported=True)
        self.assertIn("state.error_contexts.length!==0", driver)
        self.assertIn("state.sources_total===1", driver)
        self.assertIn("state.subscription_interval_seconds===86400", driver)

    def test_unknown_case_and_locale_refused(self) -> None:
        with self.assertRaises(KeyError):
            production_driver("invalid_input", "ru")
        with self.assertRaises(ValueError):
            production_driver("interval", "unknown")


if __name__ == "__main__":
    unittest.main()
