#include "market_state.h"
#include "numeric.h"

#include <cassert>
#include <iostream>

using namespace eng;

static void expect_true(bool value, const char* label) {
  if (!value) {
    std::cerr << "FAIL: " << label << "\n";
    std::exit(1);
  }
}

static void expect_eq(int64_t actual, int64_t expected, const char* label) {
  if (actual != expected) {
    std::cerr << "FAIL: " << label << " actual=" << actual << " expected=" << expected << "\n";
    std::exit(1);
  }
}

template<typename Fn>
static void expect_throws(Fn&& fn, const char* label) {
  try {
    fn();
  } catch (const std::exception&) {
    return;
  }
  std::cerr << "FAIL (expected throw): " << label << "\n";
  std::exit(1);
}

int main() {
  // Exact parsing and round-trip (mirrors Rust fixed.rs tests).
  expect_eq(parse_scaled("0", 6), 0, "parse 0");
  expect_eq(parse_scaled("1", 6), 1'000'000, "parse 1");
  expect_eq(parse_scaled("0.08", 6), 80'000, "parse 0.08");
  expect_eq(parse_scaled("-54.00", 6), -54'000'000, "parse -54");
  expect_eq(parse_scaled("0.123456789", 9), 123'456'789, "parse scale 9");
  expect_eq(parse_scaled("-0.000000001", 9), -1, "parse -1e-9");
  expect_throws([] { parse_scaled("1.0000001", 6); }, "reject 7 decimals at scale 6");
  expect_throws([] { parse_scaled("1e-3", 6); }, "reject exponent");
  expect_throws([] { parse_scaled("1", 19); }, "reject scale 19");
  expect_eq(parse_quantity_scaled("1.5", 6), 1'500'000, "parse quantity");
  expect_throws([] { parse_quantity_scaled("-1", 6); }, "reject negative quantity");

  // Rescaling is exact or throws.
  expect_eq(rescale_exact(12'300, 4, 6), 1'230'000, "rescale up");
  expect_eq(rescale_exact(1'230'000, 6, 4), 12'300, "rescale down exact");
  expect_throws([] { rescale_exact(1, 6, 4); }, "rescale down inexact");
  expect_throws([] { rescale_exact(INT64_MAX, 0, 6); }, "rescale overflow");

  // Formatting round-trips.
  expect_true(format_scaled(80'000, 6) == "0.08", "format 0.08");
  expect_true(format_scaled(-1, 9) == "-0.000000001", "format -1e-9");
  for (const char* text : {"0.000000001", "-12345.678901234", "9000000000.123456789"}) {
    int64_t value = parse_scaled(text, 9);
    expect_eq(parse_scaled(format_scaled(value, 9), 9), value, "round-trip");
  }

  // Ratio exactness, no floating point.
  Ratio multiplier(5, 2);
  expect_eq(multiplier.apply_exact(10), 25, "ratio 5/2 * 10");
  expect_throws([&] { multiplier.apply_exact(3); }, "ratio inexact");
  expect_throws([] { Ratio(1, 0); }, "ratio zero denominator");
  expect_throws([&] { multiplier.apply_exact(INT64_MAX); }, "ratio overflow");
  // INT64_MIN boundaries: negating INT64_MIN would be undefined behavior.
  Ratio min_ratio(INT64_MIN, 2);
  expect_eq(min_ratio.numerator, INT64_MIN / 2, "INT64_MIN/2 normalizes via unsigned gcd");
  expect_eq(min_ratio.denominator, 1, "INT64_MIN/2 denominator");
  expect_throws([] { Ratio(1, INT64_MIN); }, "denominator INT64_MIN rejected");
  expect_throws([] { Ratio(INT64_MIN, -1); },
                "numerator INT64_MIN with negative denominator rejected");
  Ratio unit(INT64_MIN, 1);
  expect_eq(unit.numerator, INT64_MIN, "INT64_MIN/1 numerator");
  expect_throws([&] { unit.apply_exact(2); }, "INT64_MIN * 2 overflows");
  expect_eq(Ratio(1, 1).apply_exact(INT64_MIN), INT64_MIN, "identity at INT64_MIN");
  expect_eq(min_ratio.apply_exact(2), INT64_MIN, "exact at lower bound");

  // Order book: signed prices, deltas, overflow.
  OrderBook book;
  book.apply(Side::Buy, Price(-2'500'000), QuantityChange::Abs(Quantity(10)));
  book.apply(Side::Buy, Price(INT64_MIN), QuantityChange::Abs(Quantity(1)));
  book.apply(Side::Buy, Price(INT64_MAX), QuantityChange::Abs(Quantity(1)));
  auto bids = book.bids();
  expect_true(bids.front().price.raw == INT64_MAX, "best bid is max price");
  book.apply(Side::Buy, Price(500'000), QuantityChange::Abs(Quantity(100)));
  book.apply(Side::Buy, Price(500'000), QuantityChange::Delta(QuantityDelta(-10)));
  expect_throws(
      [&] { book.apply(Side::Buy, Price(500'000), QuantityChange::Delta(QuantityDelta(-91))); },
      "negative result rejected");
  OrderBook overflow_book;
  overflow_book.apply(Side::Sell, Price(1), QuantityChange::Abs(Quantity(INT64_MAX)));
  expect_throws(
      [&] { overflow_book.apply(Side::Sell, Price(1), QuantityChange::Delta(QuantityDelta(1))); },
      "delta overflow rejected");
  // Zero quantity removes the level.
  OrderBook removal;
  removal.apply(Side::Buy, Price(1), QuantityChange::Abs(Quantity(5)));
  removal.apply(Side::Buy, Price(1), QuantityChange::Delta(QuantityDelta(-5)));
  expect_true(removal.is_empty(), "zero removes level");

  // Tick schedules across bands, with origins.
  NumericSpec spec;
  spec.tick_schedule = {
      PriceBand{Price(0), Price(100'000), Price(1'000), Price(0)},
      PriceBand{Price(100'000), Price(1'000'000), Price(10'000), Price(0)},
  };
  spec.validate();
  expect_true(spec.accepts_price(Price(99'000)), "accept lower band");
  expect_true(spec.accepts_price(Price(100'000)), "accept boundary");
  expect_true(!spec.accepts_price(Price(99'500)), "reject off grid");
  expect_true(!spec.accepts_price(Price(1'000'500)), "reject above range");
  // Non-zero origin: ticks at 1000, 3000, 5000, ...
  NumericSpec shifted;
  shifted.tick_schedule = {PriceBand{Price(1'000), Price(9'000), Price(2'000), Price(1'000)}};
  shifted.validate();
  expect_true(shifted.accepts_price(Price(3'000)), "shifted origin accept");
  expect_true(!shifted.accepts_price(Price(2'000)), "shifted origin reject");

  // Quantity steps and lot sizes.
  NumericSpec lot;
  lot.has_quantity_step = true;
  lot.quantity_step = Quantity(10);
  lot.has_lot_size = true;
  lot.lot_size = Quantity(100);
  lot.validate();
  expect_true(lot.accepts_quantity(Quantity(110)), "accept step multiple");
  expect_true(!lot.accepts_quantity(Quantity(105)), "reject non-multiple");
  expect_true(!lot.accepts_quantity(Quantity(50)), "reject below lot");

  std::cout << "all C++ numeric/orderbook checks passed\n";
  return 0;
}
