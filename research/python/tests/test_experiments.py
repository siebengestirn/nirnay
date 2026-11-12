import unittest
from quantlab.experiments import ExperimentError, content_hash, freeze_experiment, validate_experiment


def record():
    return {
        "experiment_id": "EXP-0001",
        "hypothesis": "A pre-registered example hypothesis, not an assertion of alpha.",
        "economic_rationale": "To be reviewed before the experiment begins.",
        "dataset_ref": "fixture:nse-cash:2026-11-11",
        "dataset_manifest_sha256": "a" * 64,
        "dataset_point_in_time": True,
        "code_revision": "test-revision-0001",
        "config": {"lookback": 20},
        "status": "registered",
        "registered_at_utc": "2026-11-12T00:00:00Z",
        "falsification_rule": "Reject if net out-of-sample expectancy is not positive after costs.",
    }


class ExperimentTests(unittest.TestCase):
    def test_valid_record_freezes_deterministically(self):
        a, ah = freeze_experiment(record())
        b, bh = freeze_experiment(record())
        self.assertEqual(a, b)
        self.assertEqual(ah, bh)
        self.assertEqual(len(ah), 64)

    def test_missing_fields_fail(self):
        r = record(); del r["falsification_rule"]
        with self.assertRaises(ExperimentError): validate_experiment(r)

    def test_naive_timestamp_fails(self):
        r = record(); r["registered_at_utc"] = "2026-11-12T00:00:00"
        with self.assertRaises(ExperimentError): validate_experiment(r)

    def test_bad_manifest_hash_fails(self):
        r = record(); r["dataset_manifest_sha256"] = "not-a-hash"
        with self.assertRaises(ExperimentError): validate_experiment(r)

    def test_leaked_dataset_cannot_be_candidate(self):
        r = record(); r["dataset_point_in_time"] = False; r["status"] = "candidate"
        r["result"] = {}; r["validation"] = {}
        with self.assertRaises(ExperimentError): validate_experiment(r)

    def test_canonical_hash_ignores_mapping_order(self):
        self.assertEqual(content_hash({"a": 1, "b": 2}), content_hash({"b": 2, "a": 1}))

    def test_timestamp_must_be_string_and_utc(self):
        for timestamp in (123, "2026-11-12T00:00:00+05:30"):
            r = record(); r["registered_at_utc"] = timestamp
            with self.subTest(timestamp=timestamp):
                with self.assertRaises(ExperimentError):
                    validate_experiment(r)

    def test_non_finite_configuration_values_fail(self):
        r = record(); r["config"] = {"threshold": float("nan")}
        with self.assertRaises(ExperimentError):
            freeze_experiment(r)

    def test_non_string_nested_keys_fail(self):
        r = record(); r["config"] = {1: "lookback"}
        with self.assertRaises(ExperimentError):
            validate_experiment(r)

    def test_unknown_record_fields_fail(self):
        r = record(); r["unregistered_field"] = "not in schema"
        with self.assertRaises(ExperimentError):
            validate_experiment(r)

    def test_optional_result_and_validation_must_be_objects(self):
        r = record(); r["result"] = "not-an-object"
        with self.assertRaises(ExperimentError):
            validate_experiment(r)


if __name__ == "__main__":
    unittest.main()
