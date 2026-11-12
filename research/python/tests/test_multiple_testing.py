import unittest
from quantlab.multiple_testing import holm_adjust


class HolmTests(unittest.TestCase):
    def test_adjusted_values_are_mapped_back_to_input_order(self):
        self.assertEqual(holm_adjust([0.04, 0.01, 0.03]), [0.06, 0.03, 0.06])

    def test_empty_and_singleton_inputs(self):
        self.assertEqual(holm_adjust([]), [])
        self.assertEqual(holm_adjust([0.2]), [0.2])

    def test_invalid_p_values_fail(self):
        for values in ([-0.1], [1.1], [float("nan")]):
            with self.assertRaises(ValueError):
                holm_adjust(values)


if __name__ == "__main__":
    unittest.main()
