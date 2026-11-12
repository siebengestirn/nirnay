#include "sbe.h"

#include <fstream>
#include <iostream>

// Canonical dump: one line per message. Rust prints the same format.
// Usage: eng_sbe_dump <capture.sbe> <chunk_size>

using namespace eng;

int main(int argc, char** argv) {
  if (argc < 3) {
    std::cerr << "usage: eng_sbe_dump <capture.sbe> <chunk_size>\n";
    return 1;
  }
  std::ifstream file(argv[1], std::ios::binary);
  std::vector<uint8_t> bytes((std::istreambuf_iterator<char>(file)),
                             std::istreambuf_iterator<char>());
  size_t chunk = static_cast<size_t>(std::stoul(argv[2]));
  auto messages = decode_capture(bytes, chunk);
  for (const auto& message : messages) {
    if (message.template_id == 1) {
      std::cout << "snapshot " << message.symbol << ":" << message.received_at_us << ":"
                << message.exchange_timestamp_us << ":bids=";
      bool first = true;
      for (const auto& level : message.bids) {
        if (!first)
          std::cout << ",";
        first = false;
        std::cout << level.price << ":" << level.quantity;
      }
      std::cout << ":asks=";
      first = true;
      for (const auto& level : message.asks) {
        if (!first)
          std::cout << ",";
        first = false;
        std::cout << level.price << ":" << level.quantity;
      }
      std::cout << "\n";
    } else {
      std::cout << "update " << message.symbol << ":" << message.received_at_us << ":"
                << message.exchange_timestamp_us << ":side=" << static_cast<int>(*message.side)
                << ":price=" << *message.price_level << ":qty=" << *message.quantity << "\n";
    }
  }
  return 0;
}
