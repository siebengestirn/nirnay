import unittest
from quantlab.walk_forward import expanding_splits


class WalkForwardTests(unittest.TestCase):
    def test_windows_are_forward_only_and_expand(self):
        splits = expanding_splits(1000, n_splits=3, min_train_fraction=0.4, test_fraction=0.1, embargo=5)
        self.assertEqual(len(splits), 3)
        self.assertEqual(splits[0].train_start, 0)
        self.assertLess(splits[0].train_end, splits[0].test_start)
        self.assertLess(splits[0].test_end, splits[1].test_end)
        self.assertEqual(splits[1].train_end, splits[0].test_end)

    def test_impossible_splits_fail(self):
        with self.assertRaises(ValueError):
            expanding_splits(10, n_splits=4, min_train_fraction=0.7, test_fraction=0.2)

    def test_invalid_arguments_fail(self):
        with self.assertRaises(ValueError): expanding_splits(0, n_splits=1)
        with self.assertRaises(ValueError): expanding_splits(100, n_splits=1, embargo=-1)


if __name__ == "__main__":
    unittest.main()
