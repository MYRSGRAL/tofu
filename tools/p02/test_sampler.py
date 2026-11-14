"""Канарейка FD и реальные counters внешнего sampler."""

import os
import unittest

from connected_idle import sample


class SamplerTest(unittest.TestCase):
    def test_retained_descriptors_are_visible(self) -> None:
        """Sampler обязан видеть реальную прибавку удержанных FD."""
        before = sample(os.getpid())
        handles = [open("/dev/null", "rb") for _ in range(7)]
        try:
            during = sample(os.getpid())
            self.assertEqual(during["fd"] - before["fd"], 7)
            self.assertEqual(before["starttime"], during["starttime"])
            self.assertGreaterEqual(during["cpu_ticks"], before["cpu_ticks"])
            self.assertIsNone(during["wakeups"])
            self.assertTrue(during["wakeups_reason"])
        finally:
            for handle in handles:
                handle.close()
        self.assertEqual(sample(os.getpid())["fd"], before["fd"])


if __name__ == "__main__":
    unittest.main()
