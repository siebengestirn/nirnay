#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

namespace eng {

// Challenge SBE schema
// (sbe_schemas/engineering_challenge/challenge_orderbook.yaml): schemaId=1,
// version=2, Snapshot template=1, PriceLevelUpdate template=2. The challenge
// schema is the compatibility oracle; the internal v1 contract is a separate
// protocol and is not handled here.

struct SbeLevel {
  int64_t price = 0;
  int64_t quantity = 0;
  bool operator==(const SbeLevel&) const = default;
};

// Canonical message representation shared by the Rust and C++ decoders.
struct SbeMessage {
  uint16_t template_id = 0;
  uint64_t received_at_us = 0;
  uint64_t exchange_timestamp_us = 0;
  std::string symbol;
  std::vector<SbeLevel> bids;
  std::vector<SbeLevel> asks;
  // PriceLevelUpdate only:
  std::optional<int64_t> price_level;
  std::optional<int64_t> quantity;
  std::optional<uint8_t> side;  // 0=buy, 1=sell (SideEncoding)
  bool operator==(const SbeMessage&) const = default;
};

struct DecodeResult {
  SbeMessage message;
  size_t bytes_consumed = 0;
};

// Decode one message at `offset`. Throws std::invalid_argument on any
// structural defect: truncated header/block/group/var-data, unknown
// template, wrong schema/version, invalid SideEncoding, bad lengths.
DecodeResult decode_message(const std::vector<uint8_t>& buffer, size_t offset);

// Encode one message to its canonical bytes (header + block + groups + var
// data).
std::vector<uint8_t> encode_message(const SbeMessage& message);

// Decode an aligned capture: messages never tear across chunk boundaries;
// zero padding to the chunk end terminates a chunk. Returns all messages.
std::vector<SbeMessage> decode_capture(const std::vector<uint8_t>& bytes, size_t chunk_size);

}  // namespace eng
