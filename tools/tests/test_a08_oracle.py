"""IO oracle обязан отличать настоящий canary от запрещённого actual чтения."""

import unittest

from tools.a08_report import oracle


class OracleTests(unittest.TestCase):
    def test_sensitivity_and_forbidden_inputs(self) -> None:
        canary = [{"class": "cmdline"}, {"class": "environ"}]
        actual = [{"class": "stat"}, {"class": "exe"}]
        self.assertEqual(oracle(canary, actual), {"stat": 1, "exe": 1})
        for forbidden in ("cmdline", "environ"):
            with self.assertRaisesRegex(ValueError, "запрещённый"):
                oracle(canary, actual + [{"class": forbidden}])
        with self.assertRaisesRegex(ValueError, "чувствительности"):
            oracle([], actual)
        with self.assertRaisesRegex(ValueError, "canary"):
            oracle(canary, [])
