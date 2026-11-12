// Conformance driver: reads line-oriented operations, applies them to an
// order book, and prints the resulting book as JSON on stdout.
// Format per line (space separated):
//   abs <side:buy|sell> <price> <quantity>
//   delta <side:buy|sell> <price> <delta>
//   snapshot
// Output is one JSON object: {"bids":[...],"asks":[...]} with price/quantity
// as integers, best-first order. Both Rust and C++ drivers implement this
// exact format so outputs can be diffed byte-for-byte.

#include "market_state.h"

#include <iostream>
#include <sstream>
#include <string>

using namespace eng;

int main() {
  OrderBook book;
  std::string line;
  while (std::getline(std::cin, line)) {
    std::istringstream stream(line);
    std::string op;
    if (!(stream >> op))
      continue;
    if (op == "abs" || op == "delta") {
      std::string side_text;
      int64_t price = 0;
      int64_t value = 0;
      stream >> side_text >> price >> value;
      Side side = (side_text == "buy") ? Side::Buy : Side::Sell;
      if (op == "abs") {
        book.apply(side, Price(price), QuantityChange::Abs(Quantity(value)));
      } else {
        book.apply(side, Price(price), QuantityChange::Delta(QuantityDelta(value)));
      }
    } else if (op == "snapshot") {
      auto bids = book.bids();
      auto asks = book.asks();
      std::cout << "{\"bids\":[";
      bool first = true;
      for (const auto& level : bids) {
        if (!first)
          std::cout << ",";
        first = false;
        std::cout << "{\"price\":" << level.price.raw << ",\"quantity\":" << level.quantity.raw
                  << "}";
      }
      std::cout << "],\"asks\":[";
      first = true;
      for (const auto& level : asks) {
        if (!first)
          std::cout << ",";
        first = false;
        std::cout << "{\"price\":" << level.price.raw << ",\"quantity\":" << level.quantity.raw
                  << "}";
      }
      std::cout << "]}\n";
    } else {
      std::cerr << "unknown op: " << op << "\n";
      return 1;
    }
  }
  return 0;
}
