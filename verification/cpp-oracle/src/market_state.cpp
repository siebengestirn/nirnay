#include "market_state.h"

#include <limits>
#include <optional>

namespace eng {

void OrderBook::apply(Side side, Price price, const QuantityChange& change) {
  auto& book = (side == Side::Buy) ? bids_ : asks_;
  if (!change.is_delta) {
    if (change.absolute.raw == 0) {
      book.erase(price);
    } else {
      book[price] = change.absolute;
    }
    return;
  }
  int64_t current = book.count(price) ? book[price].raw : 0;
  __int128 next = static_cast<__int128>(current) + static_cast<__int128>(change.delta.raw);
  if (next > static_cast<__int128>(std::numeric_limits<int64_t>::max())) {
    throw BookError("quantity overflow");
  }
  if (next < 0)
    throw BookError("delta would make quantity negative");
  if (next == 0) {
    book.erase(price);
  } else {
    book[price] = Quantity(static_cast<int64_t>(next));
  }
}

bool OrderBook::is_crossed() const {
  if (bids_.empty() || asks_.empty())
    return false;
  Price best_bid = bids_.rbegin()->first;
  Price best_ask = asks_.begin()->first;
  return best_bid.raw > best_ask.raw;
}

std::vector<Level> OrderBook::bids() const {
  std::vector<Level> out;
  for (auto it = bids_.rbegin(); it != bids_.rend(); ++it)
    out.push_back({it->first, it->second});
  return out;
}

std::vector<Level> OrderBook::asks() const {
  std::vector<Level> out;
  for (const auto& [price, quantity] : asks_)
    out.push_back({price, quantity});
  return out;
}

void PriceBand::validate() const {
  if (start.raw > end.raw)
    throw std::invalid_argument("band start exceeds end");
  if (step.raw <= 0)
    throw std::invalid_argument("non-positive band step");
  auto on_grid = [this](Price value) {
    __int128 offset = static_cast<__int128>(value.raw) - static_cast<__int128>(origin.raw);
    return offset % static_cast<__int128>(step.raw) == 0;
  };
  if (!on_grid(start))
    throw std::invalid_argument("band start not on grid");
  if (!on_grid(end))
    throw std::invalid_argument("band end not on grid");
}

bool PriceBand::is_on_grid(Price price) const {
  if (price.raw < start.raw || price.raw > end.raw)
    return false;
  __int128 offset = static_cast<__int128>(price.raw) - static_cast<__int128>(origin.raw);
  return offset % static_cast<__int128>(step.raw) == 0;
}

void NumericSpec::validate() const {
  if (price_scale > kMaxScale || quantity_scale > kMaxScale) {
    throw std::invalid_argument("scale exceeds maximum 18");
  }
  if (price_unit.empty() || quantity_unit.empty()) {
    throw std::invalid_argument("empty unit");
  }
  std::optional<int64_t> previous_end;
  for (const auto& band : tick_schedule) {
    band.validate();
    if (previous_end.has_value() && band.start.raw < previous_end.value()) {
      throw std::invalid_argument("price bands overlap");
    }
    previous_end = band.end.raw;
  }
  if (has_quantity_step && quantity_step.raw == 0) {
    throw std::invalid_argument("zero quantity step");
  }
  if (has_lot_size) {
    if (lot_size.raw == 0)
      throw std::invalid_argument("zero lot size");
    if (has_quantity_step && lot_size.raw % quantity_step.raw != 0) {
      throw std::invalid_argument("lot size is not a multiple of step");
    }
  }
  if (has_multiplier && multiplier.numerator == 0) {
    throw std::invalid_argument("zero multiplier");
  }
}

bool NumericSpec::accepts_price(Price price) const {
  if (tick_schedule.empty())
    return true;
  for (const auto& band : tick_schedule) {
    if (band.is_on_grid(price))
      return true;
  }
  return false;
}

bool NumericSpec::accepts_quantity(Quantity quantity) const {
  if (has_quantity_step && quantity.raw % quantity_step.raw != 0)
    return false;
  if (has_lot_size && quantity.raw != 0 && quantity.raw < lot_size.raw)
    return false;
  return true;
}

}  // namespace eng
