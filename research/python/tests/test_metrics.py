import math
import unittest
from quantlab.metrics import summarize_returns


class MetricsTests(unittest.TestCase):
    def test_metrics_are_computed_from_periodic_returns(self):
        result = summarize_returns([0.01, -0.005, 0.02, -0.01], 252)
        self.assertEqual(result["observations"], 4)
        self.assertGreater(result["cumulative_return"], 0)
        self.assertGreaterEqual(result["max_drawdown"], 0)
        self.assertTrue(math.isfinite(result["annualized_sharpe"]))

    def test_empty_or_nonfinite_returns_fail(self):
        with self.assertRaises(ValueError): summarize_returns([], 252)
        with self.assertRaises(ValueError): summarize_returns([float("nan")], 252)

    def test_invalid_period_count_fails(self):
        with self.assertRaises(ValueError): summarize_returns([0.01], 0)


if __name__ == "__main__":
    unittest.main()
