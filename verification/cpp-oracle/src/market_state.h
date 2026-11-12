#pragma once

#include "numeric.h"

#include <cstdint>
#include <map>
#include <vector>

namespace eng {

enum class Side {
  Buy,
  Sell
};

struct Level {
  Price price;
  Quantity quantity;
  bool operator==(const Level&) const = default;
};

struct QuantityChange {
  bool is_delta = false;
  Quantity absolute;
  QuantityDelta delta;
  static QuantityChange Abs(Quantity quantity) {
    return QuantityChange{false, quantity, QuantityDelta(0)};
  }
  static QuantityChange Delta(QuantityDelta delta) {
    return QuantityChange{true, Quantity(0), delta};
  }
};

class BookError : public std::exception {
public:
  explicit BookError(const char* message) : message_(message) {}
  const char* what() const noexcept override {
    return message_;
  }

private:
  const char* message_;
};

// In-memory aggregated book. Prices are signed and unrestricted: tick
// validation belongs to NumericSpec, not to the shared book. Mirrors the
// Rust OrderBook semantics exactly (checked arithmetic, zero removes).
class OrderBook {
public:
  void apply(Side side, Price price, const QuantityChange& change);
  [[nodiscard]] bool is_empty() const {
    return bids_.empty() && asks_.empty();
  }
  [[nodiscard]] bool is_crossed() const;
  [[nodiscard]] std::vector<Level> bids() const;
  [[nodiscard]] std::vector<Level> asks() const;

private:
  std::map<Price, Quantity> bids_;
  std::map<Price, Quantity> asks_;
};

// Instrument numeric specification: scales, units, tick schedule, increments.
struct PriceBand {
  Price start;
  Price end;
  Price step;
  Price origin;
  void validate() const;
  [[nodiscard]] bool is_on_grid(Price price) const;
};

struct NumericSpec {
  uint8_t price_scale = 6;
  uint8_t quantity_scale = 6;
  std::string price_unit = "USD";
  std::string quantity_unit = "contracts";
  std::vector<PriceBand> tick_schedule;
  bool has_quantity_step = false;
  Quantity quantity_step;
  bool has_lot_size = false;
  Quantity lot_size;
  bool has_multiplier = false;
  Ratio multiplier;
  void validate() const;
  [[nodiscard]] bool accepts_price(Price price) const;
  [[nodiscard]] bool accepts_quantity(Quantity quantity) const;
};

}  // namespace eng
