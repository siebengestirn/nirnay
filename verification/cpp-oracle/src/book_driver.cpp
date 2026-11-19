// Active book conformance driver. Reads the line-oriented operation corpus
// shared with the Rust driver and writes one canonical JSON snapshot per
// `snapshot` operation. It is a verification target, not a runtime component.

#include "market_state.h"

#include <cstdint>
#include <exception>
#include <iostream>
#include <sstream>
#include <string>

namespace {

void print_levels(const std::vector<eng::Level>& levels, std::ostream& out) {
  bool first = true;
  for (const auto& level : levels) {
    if (!first) {
      out << ",";
    }
    first = false;
    out << "{\"price\":" << level.price.raw
        << ",\"quantity\":" << level.quantity.raw << "}";
  }
}

}  // namespace

int main() {
  eng::OrderBook book;
  std::string line;
  while (std::getline(std::cin, line)) {
    std::istringstream stream(line);
    std::string op;
    if (!(stream >> op)) {
      continue;
    }
    if (op == "abs" || op == "delta") {
      std::string side_text;
      int64_t price = 0;
      int64_t value = 0;
      stream >> side_text >> price >> value;
      const eng::Side side = side_text == "buy" ? eng::Side::Buy : eng::Side::Sell;
      try {
        if (op == "abs") {
          book.apply(side, eng::Price(price),
                     eng::QuantityChange::Abs(eng::Quantity(value)));
        } else {
          book.apply(side, eng::Price(price),
                     eng::QuantityChange::Delta(eng::QuantityDelta(value)));
        }
      } catch (const std::exception& error) {
        std::cerr << "book error: " << error.what() << "\n";
        return 1;
      }
    } else if (op == "snapshot") {
      std::cout << "{\"bids\":[";
      print_levels(book.bids(), std::cout);
      std::cout << "],\"asks\":[";
      print_levels(book.asks(), std::cout);
      std::cout << "]}\n";
    } else {
      std::cerr << "unknown op: " << op << "\n";
      return 1;
    }
  }
  return 0;
}
