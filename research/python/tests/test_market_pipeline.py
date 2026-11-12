from datetime import date
from decimal import Decimal
import tempfile
import unittest
from pathlib import Path

from quantlab.market import MarketEvent, BookLevel
from quantlab.providers import UpstoxV3Normalizer, FyersV3Normalizer
from quantlab.reconcile import reconcile_pair
from quantlab.point_in_time import (PITListing, CorporateAction, PITReferenceData,
                                    PITUniverseMembership, PITDatasetBuilder, write_dataset,
                                    verify_published_dataset)


class ProviderTests(unittest.TestCase):
    def test_upstox_normalizes_market_ff_and_keeps_timestamp_semantics(self):
        msg = {"type": "live_feed", "currentTs": "1794372000000", "feeds": {
            "NSE_EQ|INE002A01018": {"fullFeed": {"marketFF": {
                "ltpc": {"ltp": 600.5, "ltt": "1794371999000", "ltq": "4", "cp": 590},
                "marketLevel": {"bidAskQuote": [
                    {"bidP": 600.4, "bidQ": "20", "askP": 600.6, "askQ": "30"},
                    {"bidP": 600.3, "bidQ": "50", "askP": 600.7, "askQ": "40"}]},
                "vtt": "12345"}}}}}
        events = UpstoxV3Normalizer({"NSE_EQ|INE002A01018": "in.ine002a01018"}).normalize(
            msg, received_utc_ns=1794372000000000000, local_sequence=4)
        self.assertEqual(len(events), 1)  # First LTP snapshot seeds state; not a trade print.
        event = events[0]
        self.assertEqual(event.instrument_id, "in.ine002a01018")
        self.assertEqual(event.bids[0].price, Decimal("600.4"))
        self.assertEqual(event.asks[0].quantity, 30)
        self.assertEqual(event.source_timestamp_ns, 1794372000000000000)
        self.assertEqual(event.source_timestamp_semantics, "upstox_currentTs_feed_envelope_epoch_ms")
        self.assertEqual(event.last_trade_time_ns, 1794371999000000000)
        self.assertEqual(event.last_trade_time_semantics, "upstox_ltpc_ltt_epoch_ms")

    def test_fyers_symbol_and_depth_are_distinct_shapes(self):
        normalizer = FyersV3Normalizer({"NSE:SBIN-EQ": "in.sbin"})
        quote = normalizer.normalize({"type": "sf", "symbol": "NSE:SBIN-EQ", "ltp": 100.2,
            "exch_feed_time": 1794371999, "last_traded_time": 1794371998,
            "bid_price": 100.15, "bid_size": 10, "ask_price": 100.25,
            "ask_size": 15, "last_traded_qty": 2, "vol_traded_today": 1000},
            received_utc_ns=1794372000000000000, local_sequence=5)
        depth = normalizer.normalize({"type": "dp", "symbol": "NSE:SBIN-EQ",
            "bid_price1": 100.15, "bid_size1": 10, "bid_order1": 2,
            "ask_price1": 100.25, "ask_size1": 15, "ask_order1": 3},
            received_utc_ns=1794372000000000000, local_sequence=6)
        self.assertEqual(quote.event_kind, "quote")
        self.assertEqual(depth.event_kind, "depth")
        self.assertEqual(depth.bids[0].order_count, 2)
        self.assertIsNone(depth.source_timestamp_ns)  # DepthUpdate lacks exch_feed_time in this fixture.
        self.assertIsNone(normalizer.normalize({"type": "cn"}, received_utc_ns=1))

    def test_upstox_emits_new_last_trade_observation_not_on_repeated_snapshot(self):
        normalizer = UpstoxV3Normalizer({"NSE_EQ|X": "in.x"})
        def payload(ts, qty, volume):
            market = {
                "ltpc": {"ltp": "100.25", "ltt": str(ts), "ltq": str(qty)},
                "marketLevel": {"bidAskQuote": [
                    {"bidP": "100.2", "bidQ": "10", "askP": "100.3", "askQ": "10"}
                ]},
                "vtt": str(volume),
            }
            return {"currentTs": str(ts + 1), "feeds": {
                "NSE_EQ|X": {"fullFeed": {"marketFF": market}}
            }}
        first = normalizer.normalize(payload(1794371999000, 2, 100), received_utc_ns=1794372000000000000)
        repeated = normalizer.normalize(payload(1794371999000, 2, 100), received_utc_ns=1794372001000000000)
        advanced = normalizer.normalize(payload(1794372000000, 3, 103), received_utc_ns=1794372002000000000)
        self.assertEqual([x.event_kind for x in first], ["depth"])
        self.assertEqual([x.event_kind for x in repeated], ["depth"])
        self.assertEqual([x.event_kind for x in advanced], ["depth", "last_trade_observation"])
        self.assertIsNone(advanced[-1].aggressor_side)
        self.assertIn("aggressor_side_unknown", advanced[-1].quality_flags)

    def test_fyers_emits_trade_only_after_last_trade_tuple_advances(self):
        normalizer = FyersV3Normalizer({"NSE:SBIN-EQ": "in.sbin"})
        base = {"type": "sf", "symbol": "NSE:SBIN-EQ", "ltp": 100.2,
                "exch_feed_time": 1794371999, "last_traded_time": 1794371998,
                "bid_price": 100.15, "bid_size": 10, "ask_price": 100.25,
                "ask_size": 15, "last_traded_qty": 2, "vol_traded_today": 1000}
        first = normalizer.normalize_observations(base, received_utc_ns=1794372000000000000)
        repeated = normalizer.normalize_observations(base, received_utc_ns=1794372001000000000)
        advanced = dict(base, exch_feed_time=1794372000, last_traded_time=1794371999,
                        vol_traded_today=1002, last_traded_qty=2)
        third = normalizer.normalize_observations(advanced, received_utc_ns=1794372002000000000)
        self.assertEqual([x.event_kind for x in first], ["quote"])
        self.assertEqual([x.event_kind for x in repeated], ["quote"])
        self.assertEqual([x.event_kind for x in third], ["quote", "last_trade_observation"])


    def test_upstox_timestamp_regression_does_not_rewind_trade_deduplication(self):
        normalizer = UpstoxV3Normalizer({"NSE_EQ|X": "in.x"})
        def payload(ts, volume):
            return {"currentTs": str(ts + 1), "feeds": {"NSE_EQ|X": {"fullFeed": {"marketFF": {
                "ltpc": {"ltp": "100", "ltt": str(ts), "ltq": "2"},
                "marketLevel": {"bidAskQuote": [{"bidP": "99.9", "bidQ": "10", "askP": "100.1", "askQ": "10"}]},
                "vtt": str(volume)}}}}}
        first = normalizer.normalize(payload(1000, 100), received_utc_ns=2_000_000_000)
        regressed = normalizer.normalize(payload(900, 90), received_utc_ns=2_100_000_000)
        recovered = normalizer.normalize(payload(1100, 110), received_utc_ns=2_200_000_000)
        self.assertNotIn("last_trade_observation", [x.event_kind for x in first])
        self.assertIn("trade_timestamp_regression", regressed[0].quality_flags)
        self.assertEqual([x.event_kind for x in recovered], ["depth", "last_trade_observation"])

    def test_fyers_timestamp_regression_does_not_rewind_trade_deduplication(self):
        normalizer = FyersV3Normalizer({"NSE:SBIN-EQ": "in.sbin"})
        def payload(ts, volume):
            return {"type": "sf", "symbol": "NSE:SBIN-EQ", "ltp": 100,
                "exch_feed_time": ts + 1, "last_traded_time": ts,
                "bid_price": 99.9, "bid_size": 10, "ask_price": 100.1,
                "ask_size": 10, "last_traded_qty": 2, "vol_traded_today": volume}
        first = normalizer.normalize_observations(payload(1000, 100), received_utc_ns=2_000_000_000)
        regressed = normalizer.normalize_observations(payload(900, 90), received_utc_ns=2_100_000_000)
        recovered = normalizer.normalize_observations(payload(1100, 110), received_utc_ns=2_200_000_000)
        self.assertEqual([x.event_kind for x in first], ["quote"])
        self.assertIn("trade_timestamp_regression", regressed[0].quality_flags)
        self.assertEqual([x.event_kind for x in recovered], ["quote", "last_trade_observation"])

    def test_inferred_last_trade_observation_is_not_a_confirmed_print(self):
        normalizer = FyersV3Normalizer({"NSE:SBIN-EQ": "in.sbin"})
        base = {"type": "sf", "symbol": "NSE:SBIN-EQ", "ltp": 100.2,
                "exch_feed_time": 1794371999, "last_traded_time": 1794371998,
                "bid_price": 100.15, "bid_size": 10, "ask_price": 100.25,
                "ask_size": 15, "last_traded_qty": 2, "vol_traded_today": 1000}
        normalizer.normalize_observations(base, received_utc_ns=1794372000000000000)
        advanced = dict(base, exch_feed_time=1794372000, last_traded_time=1794371999,
                        vol_traded_today=1002)
        events = normalizer.normalize_observations(advanced, received_utc_ns=1794372001000000000)
        observation = next(x for x in events if x.event_kind == "last_trade_observation")
        self.assertIn("not_a_confirmed_trade_print", observation.quality_flags)
        self.assertIsNone(observation.aggressor_side)

    def test_reconcile_latest_ignores_last_trade_observations(self):
        from quantlab.reconcile import reconcile_latest
        up = MarketEvent(1, "up-book", "upstox", "NSE_EQ|X", "in.x", "NSE", "quote",
            100, None, 2, None, None, 90, "last_trade", Decimal("100"), 1, None, None,
            (BookLevel(Decimal("99.9"), 10),), (BookLevel(Decimal("100.1"), 10),), "a"*64)
        fy = MarketEvent(1, "fy-book", "fyers", "NSE:X-EQ", "in.x", "NSE", "quote",
            100, None, 2, None, None, 90, "last_trade", Decimal("100"), 1, None, None,
            (BookLevel(Decimal("99.9"), 10),), (BookLevel(Decimal("100.1"), 10),), "b"*64)
        inferred = MarketEvent(1, "fy-ltp", "fyers", "NSE:X-EQ", "in.x", "NSE", "last_trade_observation",
            101, None, 3, None, None, 91, "last_trade", Decimal("101"), 1, None, None,
            (), (), "c"*64, ("not_a_confirmed_trade_print",))
        result = reconcile_latest([up, fy, inferred], now_ns=110)
        self.assertEqual(result[0].status, "consistent_within_tolerance")

    def test_cross_feed_reconciliation_requires_comparable_trade_timestamps(self):
        up = MarketEvent(1, "u", "upstox", "NSE_EQ|X", "in.x", "NSE", "quote",
            1_000_000_000, None, 1, None, None, 900_000_000, "last_trade", Decimal("100"),
            1, None, None, (BookLevel(Decimal("99.9"), 10),),
            (BookLevel(Decimal("100.1"), 10),), "a"*64)
        fy = MarketEvent(1, "f", "fyers", "NSE:X-EQ", "in.x", "NSE", "quote",
            1_100_000_000, None, 1, None, None, 910_000_000, "last_trade", Decimal("100"),
            1, None, None, (BookLevel(Decimal("99.9"), 10),),
            (BookLevel(Decimal("100.1"), 10),), "b"*64)
        result = reconcile_pair(up, fy, now_ns=1_200_000_000)
        self.assertEqual(result.status, "consistent_within_tolerance")
        self.assertEqual(result.price_difference_bps, 0)


class PointInTimeTests(unittest.TestCase):
    def _event(self):
        return MarketEvent(1, "u1", "upstox", "NSE_EQ|X", None, "NSE", "quote",
            1794372000000000000, None, 1, None, None, 1794371999000000000,
            "last_trade", Decimal("100"), 1, None, None,
            (BookLevel(Decimal("99.9"), 10),), (BookLevel(Decimal("100.1"), 10),), "c"*64)

    def test_dataset_uses_provider_observation_date_not_late_receive_date(self):
        from datetime import datetime
        from zoneinfo import ZoneInfo
        ist = ZoneInfo("Asia/Kolkata")
        receive_ns = int(datetime(2026, 11, 13, 0, 2, tzinfo=ist).timestamp() * 1_000_000_000)
        source_ns = int(datetime(2026, 11, 12, 15, 0, tzinfo=ZoneInfo("UTC")).timestamp() * 1_000_000_000)
        event = MarketEvent(1, "late", "upstox", "NSE_EQ|X", None, "NSE", "quote",
            receive_ns, None, 1, source_ns, "provider_feed_timestamp", None, None,
            Decimal("100"), 1, None, None, (), (), "c"*64, (), "f"*64)
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 12), date(2026, 11, 12), receive_ns - 1, "nse-master", "d"*64)
        rows, manifest = PITDatasetBuilder(PITReferenceData([listing])).build(
            [event], dataset_id="late-feed", created_at_utc_ns=receive_ns)
        self.assertEqual(rows[0].session_date_ist, "2026-11-12")
        self.assertEqual(rows[0].session_date_source, "provider_feed_timestamp")
        self.assertEqual(manifest.source_frame_hashes, ("f"*64,))
        self.assertNotIn("some_rows_lack_original_capture_frame_hash", manifest.warnings)

    def test_dataset_marks_local_receive_date_fallback_and_missing_wire_lineage(self):
        from dataclasses import replace
        event = replace(self._event(), source_timestamp_ns=None, source_timestamp_semantics=None,
                        last_trade_time_ns=None, last_trade_time_semantics=None)
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 1), None, event.received_utc_ns - 1, "nse-master", "d"*64)
        rows, manifest = PITDatasetBuilder(PITReferenceData([listing])).build(
            [event], dataset_id="fallback", created_at_utc_ns=event.received_utc_ns)
        self.assertEqual(rows[0].session_date_source, "local_receive_time_fallback")
        self.assertIn("some_rows_lack_original_capture_frame_hash", manifest.warnings)
        self.assertIn("local_receive", manifest.session_date_policy)

    def test_bitemporal_listing_and_known_split_are_preserved(self):
        now = 1794372000000000000
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 1), None, now - 1, "nse-master", "d"*64)
        action = CorporateAction("CA1", "in.x", "split", now - 10, date(2026, 11, 12),
            now - 4, now - 5, "exchange-announcement", "e"*64, 2, 1)
        refs = PITReferenceData([listing], [action])
        rows, manifest = PITDatasetBuilder(refs).build([self._event()], dataset_id="fixture", created_at_utc_ns=now)
        self.assertEqual(rows[0].split_price_factor_num, 1)
        self.assertEqual(rows[0].split_price_factor_den, 2)
        self.assertEqual(rows[0].action_lineage, ("CA1",))
        self.assertTrue(manifest.point_in_time)
        self.assertFalse(manifest.raw_prices_mutated)
        with tempfile.TemporaryDirectory() as d:
            from quantlab.point_in_time import verify_published_dataset
            destination = Path(d) / "dataset"
            write_dataset(rows, manifest, destination)
            self.assertTrue((destination / "events.jsonl").exists())
            self.assertTrue((destination / "manifest.json").exists())
            self.assertEqual(verify_published_dataset(destination)["status"], "verified_internal_integrity")

    def test_dataset_rejects_provider_listing_venue_mismatch(self):
        event = self._event()
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "BSE",
            date(2026, 11, 1), None, event.received_utc_ns - 1, "master", "a"*64)
        with self.assertRaisesRegex(ValueError, "conflicts with point-in-time listing venue"):
            PITDatasetBuilder(PITReferenceData([listing])).build(
                [event], dataset_id="venue-mismatch", created_at_utc_ns=event.received_utc_ns)

    def test_late_mapping_does_not_rewrite_historical_knowledge(self):
        event = self._event()
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 1), None, event.received_utc_ns + 1, "master", "a"*64)
        with self.assertRaisesRegex(ValueError, "missing point-in-time"):
            PITDatasetBuilder(PITReferenceData([listing])).build([event], dataset_id="x", created_at_utc_ns=event.received_utc_ns)

    def test_complex_action_requires_explicit_transform(self):
        action = CorporateAction("CA2", "in.x", "rights", 10, date(2026, 11, 12), 20, 15,
                                 "source", "f"*64, 1, 1)
        refs = PITReferenceData([], [action])
        with self.assertRaisesRegex(ValueError, "requires an approved"):
            refs.split_adjustment("in.x", observation_date=date(2026, 11, 11), decision_knowledge_ns=30)

    def test_revised_corporate_action_is_bitemporal_and_preserves_superseded_lineage(self):
        original = CorporateAction("CA1", "in.x", "split", 10, date(2026, 12, 1),
            20, 15, "exchange", "a"*64, 2, 1)
        revised = CorporateAction("CA2", "in.x", "split", 10, date(2026, 12, 5),
            40, 35, "exchange-corrigendum", "b"*64, 3, 1,
            status="revised", supersedes_action_id="CA1", record_date=date(2026, 12, 8))
        refs = PITReferenceData([], [original, revised])
        before_revision = refs.known_actions("in.x", known_at_ns=30)
        after_revision = refs.known_actions("in.x", known_at_ns=50)
        self.assertEqual([x.action_id for x in before_revision], ["CA1"])
        self.assertEqual([x.action_id for x in after_revision], ["CA2"])
        self.assertEqual(refs.action_revision_lineage(after_revision[0], known_at_ns=50), ("CA1", "CA2"))
        factor_before, _ = refs.split_adjustment("in.x", observation_date=date(2026, 11, 20), decision_knowledge_ns=30)
        factor_after, _ = refs.split_adjustment("in.x", observation_date=date(2026, 11, 20), decision_knowledge_ns=50)
        self.assertEqual(factor_before, __import__("fractions").Fraction(1, 2))
        self.assertEqual(factor_after, __import__("fractions").Fraction(1, 3))

    def test_cancelled_action_does_not_adjust_but_retains_audit_lineage(self):
        original = CorporateAction("CA1", "in.x", "bonus", 10, date(2026, 12, 1),
            20, 15, "exchange", "a"*64, 2, 1)
        cancelled = CorporateAction("CA2", "in.x", "bonus", 10, date(2026, 12, 1),
            40, 35, "exchange-cancellation", "b"*64, 2, 1,
            status="cancelled", supersedes_action_id="CA1")
        refs = PITReferenceData([], [original, cancelled])
        heads = refs.known_actions("in.x", known_at_ns=50)
        self.assertEqual([x.action_id for x in heads], ["CA2"])
        factor, lineage = refs.split_adjustment("in.x", observation_date=date(2026, 11, 20), decision_knowledge_ns=50)
        self.assertEqual(factor, __import__("fractions").Fraction(1, 1))
        self.assertEqual(lineage, ())
        self.assertEqual(refs.action_revision_lineage(heads[0], known_at_ns=50), ("CA1", "CA2"))

    def test_listing_correction_does_not_rewrite_earlier_knowledge(self):
        initial = PITListing("L1", "upstox", "NSE_EQ|X", "in.old", "NSE",
            date(2026, 11, 1), None, 10, "master-v1", "a"*64)
        corrected = PITListing("L2", "upstox", "NSE_EQ|X", "in.new", "NSE",
            date(2026, 11, 1), None, 20, "master-correction", "b"*64,
            supersedes_listing_id="L1")
        refs = PITReferenceData([initial, corrected])
        self.assertEqual(refs.resolve("upstox", "NSE_EQ|X", valid_on=date(2026, 11, 12), known_at_ns=15).instrument_id, "in.old")
        self.assertEqual(refs.resolve("upstox", "NSE_EQ|X", valid_on=date(2026, 11, 12), known_at_ns=25).instrument_id, "in.new")

    def test_point_in_time_universe_membership_filters_without_current_constituent_lookahead(self):
        event = self._event()
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 1), None, event.received_utc_ns - 10, "master", "d"*64)
        admitted = PITUniverseMembership("M1", "nifty-fixture", "in.x", date(2026, 11, 1),
            None, event.received_utc_ns - 5, "historical-index-bulletin", "e"*64)
        # This later membership correction cannot leak into an earlier point-in-time row.
        future_only = PITUniverseMembership("M2", "nifty-fixture", "in.x", date(2026, 11, 1),
            None, event.received_utc_ns + 5, "later-correction", "f"*64,
            supersedes_membership_id="M1")
        refs = PITReferenceData([listing], [], [admitted, future_only])
        rows, manifest = PITDatasetBuilder(refs).build([event], dataset_id="universe",
            created_at_utc_ns=event.received_utc_ns, universe_id="nifty-fixture")
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0].universe_membership_id, "M1")
        self.assertTrue(manifest.universe_membership_filter_applied)
        self.assertFalse(manifest.survivorship_completeness_independently_verified)

    def test_dataset_verifier_recomputes_nested_capture_frame_hashes(self):
        from dataclasses import replace
        event = replace(self._event(), source_frame_sha256="f" * 64)
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 1), None, event.received_utc_ns - 1, "master", "d"*64)
        rows, manifest = PITDatasetBuilder(PITReferenceData([listing])).build(
            [event], dataset_id="capture-frame-lineage", created_at_utc_ns=event.received_utc_ns)
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "dataset"
            write_dataset(rows, manifest, target)
            self.assertEqual(verify_published_dataset(target)["status"], "verified_internal_integrity")
            manifest_path = target / "manifest.json"
            publication = __import__("json").loads(manifest_path.read_text())
            publication["dataset"]["source_frame_hashes"] = []
            manifest_path.write_text(__import__("json").dumps(publication))
            with self.assertRaisesRegex(ValueError, "source_frame_hashes"):
                verify_published_dataset(target)

    def test_dataset_rejects_duplicate_canonical_event_ids(self):
        event = self._event()
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 1), None, event.received_utc_ns - 1, "master", "d"*64)
        with self.assertRaisesRegex(ValueError, "duplicate canonical event_id"):
            PITDatasetBuilder(PITReferenceData([listing])).build([event, event], dataset_id="dup",
                created_at_utc_ns=event.received_utc_ns)

    def test_dataset_writer_is_immutable_and_refuses_overwrite(self):
        event = self._event()
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 1), None, event.received_utc_ns - 1, "master", "d"*64)
        rows, manifest = PITDatasetBuilder(PITReferenceData([listing])).build([event], dataset_id="fixture",
            created_at_utc_ns=event.received_utc_ns)
        with tempfile.TemporaryDirectory() as d:
            target = Path(d) / "published"
            write_dataset(rows, manifest, target)
            with self.assertRaises(FileExistsError):
                write_dataset(rows, manifest, target)
            import json
            published = json.loads((target / "manifest.json").read_text())
            self.assertIn("events.jsonl", published["artifacts"])
            self.assertEqual(published["dataset"]["row_count"], 1)
            verified = verify_published_dataset(target)
            self.assertEqual(verified["status"], "verified_internal_integrity")
            self.assertFalse(verified["cryptographic_signature"])

    def test_signed_dataset_requires_trusted_key_and_verifies_detached_signature(self):
        import base64
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
        event = self._event()
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 1), None, event.received_utc_ns - 1, "master", "d"*64)
        rows, manifest = PITDatasetBuilder(PITReferenceData([listing])).build(
            [event], dataset_id="signed-fixture", created_at_utc_ns=event.received_utc_ns)
        private = Ed25519PrivateKey.generate()
        class Signer:
            public_key_b64 = base64.b64encode(private.public_key().public_bytes(
                Encoding.Raw, PublicFormat.Raw)).decode("ascii")
            @staticmethod
            def sign(message: bytes) -> bytes:
                return private.sign(message)
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "signed-dataset"
            write_dataset(rows, manifest, target, signer=Signer(), signing_key_id="research-data-2026")
            with self.assertRaisesRegex(RuntimeError, "explicit trusted_public_keys"):
                verify_published_dataset(target)
            trusted = {"research-data-2026": Signer.public_key_b64}
            verified = verify_published_dataset(target, trusted_public_keys=trusted)
            self.assertTrue(verified["cryptographic_signature"])
            self.assertTrue(verified["cryptographic_signature_verified"])
            self.assertEqual(verified["signature_key_id"], "research-data-2026")
            # Altering the signed publication manifest is rejected even if its
            # internal artifact hashes still happen to describe the same files.
            manifest_path = target / "manifest.json"
            publication = __import__("json").loads(manifest_path.read_text())
            publication["dataset"]["created_at_utc_ns"] += 1
            manifest_path.write_text(__import__("json").dumps(publication, sort_keys=True) + "\n")
            with self.assertRaisesRegex(ValueError, "manifest digest mismatch"):
                verify_published_dataset(target, trusted_public_keys=trusted)

    def test_parquet_roundtrip_is_semantically_verified_against_canonical_jsonl(self):
        import pytest
        pytest.importorskip("pyarrow")
        import hashlib
        import json
        import pyarrow as pa
        import pyarrow.parquet as pq
        event = self._event()
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 1), None, event.received_utc_ns - 1, "master", "d"*64)
        rows, manifest = PITDatasetBuilder(PITReferenceData([listing])).build(
            [event], dataset_id="parquet-semantic", created_at_utc_ns=event.received_utc_ns)
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "dataset"
            write_dataset(rows, manifest, target, parquet=True)
            verified = verify_published_dataset(target)
            self.assertTrue(verified["parquet_semantically_verified"])
            # Re-write a syntactically valid, checksummed Parquet file whose
            # data differs from JSONL. Artifact SHA alone must not pass it.
            publication_path = target / "manifest.json"
            publication = json.loads(publication_path.read_text())
            parquet_path = target / "events.parquet"
            table = pq.read_table(parquet_path)
            altered_rows = table.to_pylist()
            altered_rows[0]["session_date_ist"] = "2099-01-01"
            changed = pa.Table.from_pylist(altered_rows).replace_schema_metadata(table.schema.metadata)
            pq.write_table(changed, parquet_path, compression="zstd", version="2.6", data_page_version="2.0")
            publication["artifacts"]["events.parquet"] = hashlib.sha256(parquet_path.read_bytes()).hexdigest()
            publication_path.write_text(json.dumps(publication, sort_keys=True, separators=(",", ":")) + "\n")
            with self.assertRaisesRegex(ValueError, "semantic row differs"):
                verify_published_dataset(target)

    def test_dataset_verifier_detects_artifact_tampering(self):
        event = self._event()
        listing = PITListing("L1", "upstox", "NSE_EQ|X", "in.x", "NSE",
            date(2026, 11, 1), None, event.received_utc_ns - 1, "master", "d"*64)
        rows, manifest = PITDatasetBuilder(PITReferenceData([listing])).build([event], dataset_id="fixture",
            created_at_utc_ns=event.received_utc_ns)
        with tempfile.TemporaryDirectory() as d:
            target = Path(d) / "published"
            write_dataset(rows, manifest, target)
            with (target / "events.jsonl").open("ab") as stream:
                stream.write(b" ")
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                verify_published_dataset(target)


if __name__ == "__main__":
    unittest.main()
