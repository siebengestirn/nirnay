#include "sbe.h"

#include <stdexcept>

namespace eng {

namespace {

constexpr uint16_t kSchemaId = 1;
constexpr uint16_t kSchemaVersion = 2;
constexpr uint16_t kSnapshotTemplate = 1;
constexpr uint16_t kUpdateTemplate = 2;
constexpr size_t kHeaderBytes = 8;
constexpr size_t kSnapshotBlockBytes = 16;
constexpr size_t kUpdateBlockBytes = 33;
constexpr size_t kGroupHeaderBytes = 4;
constexpr size_t kEntryBytes = 16;  // price i64 + quantity i64

uint16_t read_u16(const std::vector<uint8_t>& b, size_t o) {
  if (o + 2 > b.size())
    throw std::invalid_argument("truncated u16");
  return static_cast<uint16_t>(b[o] | (static_cast<uint16_t>(b[o + 1]) << 8));
}

uint32_t read_u32(const std::vector<uint8_t>& b, size_t o) {
  if (o + 4 > b.size())
    throw std::invalid_argument("truncated u32");
  uint32_t v = 0;
  for (int i = 0; i < 4; ++i)
    v |= static_cast<uint32_t>(b[o + i]) << (8 * i);
  return v;
}

uint64_t read_u64(const std::vector<uint8_t>& b, size_t o) {
  if (o + 8 > b.size())
    throw std::invalid_argument("truncated u64");
  uint64_t v = 0;
  for (int i = 0; i < 8; ++i)
    v |= static_cast<uint64_t>(b[o + i]) << (8 * i);
  return v;
}

int64_t read_i64(const std::vector<uint8_t>& b, size_t o) {
  return static_cast<int64_t>(read_u64(b, o));
}

void need(const std::vector<uint8_t>& b, size_t offset, size_t length, const char* what) {
  if (offset > b.size() || length > b.size() - offset) {
    throw std::invalid_argument(std::string("truncated ") + what);
  }
}

void append_u16(std::vector<uint8_t>& out, uint16_t v) {
  out.push_back(static_cast<uint8_t>(v & 0xff));
  out.push_back(static_cast<uint8_t>((v >> 8) & 0xff));
}

void append_u32(std::vector<uint8_t>& out, uint32_t v) {
  for (int i = 0; i < 4; ++i)
    out.push_back(static_cast<uint8_t>((v >> (8 * i)) & 0xff));
}

void append_u64(std::vector<uint8_t>& out, uint64_t v) {
  for (int i = 0; i < 8; ++i)
    out.push_back(static_cast<uint8_t>((v >> (8 * i)) & 0xff));
}

void append_i64(std::vector<uint8_t>& out, int64_t v) {
  append_u64(out, static_cast<uint64_t>(v));
}

std::vector<SbeLevel> read_group(const std::vector<uint8_t>& b, size_t& pos) {
  need(b, pos, kGroupHeaderBytes, "group header");
  uint16_t entry_block = read_u16(b, pos);
  uint16_t count = read_u16(b, pos + 2);
  if (entry_block != kEntryBytes)
    throw std::invalid_argument("invalid group entry block length");
  pos += kGroupHeaderBytes;
  std::vector<SbeLevel> levels;
  levels.reserve(count);
  for (uint16_t i = 0; i < count; ++i) {
    need(b, pos, kEntryBytes, "group entry");
    levels.push_back({read_i64(b, pos), read_i64(b, pos + 8)});
    pos += kEntryBytes;
  }
  return levels;
}

std::string read_string(const std::vector<uint8_t>& b, size_t& pos) {
  need(b, pos, 4, "var-data length");
  uint32_t length = read_u32(b, pos);
  pos += 4;
  need(b, pos, length, "var-data");
  std::string text(reinterpret_cast<const char*>(b.data() + pos), length);
  pos += length;
  return text;
}

}  // namespace

DecodeResult decode_message(const std::vector<uint8_t>& buffer, size_t offset) {
  need(buffer, offset, kHeaderBytes, "message header");
  uint16_t block_length = read_u16(buffer, offset);
  uint16_t template_id = read_u16(buffer, offset + 2);
  uint16_t schema_id = read_u16(buffer, offset + 4);
  uint16_t schema_version = read_u16(buffer, offset + 6);
  if (schema_id != kSchemaId)
    throw std::invalid_argument("unknown schema id");
  if (schema_version != kSchemaVersion)
    throw std::invalid_argument("unknown schema version");
  size_t body = offset + kHeaderBytes;

  SbeMessage message;
  message.template_id = template_id;

  if (template_id == kSnapshotTemplate) {
    if (block_length != kSnapshotBlockBytes) {
      throw std::invalid_argument("snapshot block length mismatch");
    }
    need(buffer, body, block_length, "snapshot block");
    message.received_at_us = read_u64(buffer, body);
    message.exchange_timestamp_us = read_u64(buffer, body + 8);
    size_t pos = body + block_length;
    message.bids = read_group(buffer, pos);
    message.asks = read_group(buffer, pos);
    message.symbol = read_string(buffer, pos);
    return {message, pos - offset};
  }

  if (template_id == kUpdateTemplate) {
    if (block_length != kUpdateBlockBytes) {
      throw std::invalid_argument("update block length mismatch");
    }
    need(buffer, body, block_length, "update block");
    message.received_at_us = read_u64(buffer, body);
    message.exchange_timestamp_us = read_u64(buffer, body + 8);
    message.price_level = read_i64(buffer, body + 16);
    message.quantity = read_i64(buffer, body + 24);
    uint8_t side = buffer[body + 32];
    if (side > 1)
      throw std::invalid_argument("invalid SideEncoding");
    message.side = side;
    size_t pos = body + block_length;
    message.symbol = read_string(buffer, pos);
    return {message, pos - offset};
  }

  throw std::invalid_argument("unknown template id");
}

std::vector<uint8_t> encode_message(const SbeMessage& message) {
  std::vector<uint8_t> out;
  if (message.template_id == kSnapshotTemplate) {
    append_u16(out, static_cast<uint16_t>(kSnapshotBlockBytes));
    append_u16(out, kSnapshotTemplate);
    append_u16(out, kSchemaId);
    append_u16(out, kSchemaVersion);
    append_u64(out, message.received_at_us);
    append_u64(out, message.exchange_timestamp_us);
    for (const auto* side : {&message.bids, &message.asks}) {
      if (side->size() > 0xffffu) {
        throw std::invalid_argument("group size exceeds uint16 field");
      }
      append_u16(out, static_cast<uint16_t>(kEntryBytes));
      append_u16(out, static_cast<uint16_t>(side->size()));
      for (const auto& level : *side) {
        append_i64(out, level.price);
        append_i64(out, level.quantity);
      }
    }
    if (message.symbol.size() > 0xffff'ffffu) {
      throw std::invalid_argument("symbol length exceeds uint32 field");
    }
    append_u32(out, static_cast<uint32_t>(message.symbol.size()));
    out.insert(out.end(), message.symbol.begin(), message.symbol.end());
    return out;
  }
  if (message.template_id == kUpdateTemplate) {
    if (!message.price_level || !message.quantity || !message.side || *message.side > 1) {
      throw std::invalid_argument("update requires price, quantity, and valid side");
    }
    append_u16(out, static_cast<uint16_t>(kUpdateBlockBytes));
    append_u16(out, kUpdateTemplate);
    append_u16(out, kSchemaId);
    append_u16(out, kSchemaVersion);
    append_u64(out, message.received_at_us);
    append_u64(out, message.exchange_timestamp_us);
    append_i64(out, *message.price_level);
    append_i64(out, *message.quantity);
    out.push_back(*message.side);
    if (message.symbol.size() > 0xffff'ffffu) {
      throw std::invalid_argument("symbol length exceeds uint32 field");
    }
    append_u32(out, static_cast<uint32_t>(message.symbol.size()));
    out.insert(out.end(), message.symbol.begin(), message.symbol.end());
    return out;
  }
  throw std::invalid_argument("unknown template id");
}

std::vector<SbeMessage> decode_capture(const std::vector<uint8_t>& bytes, size_t chunk_size) {
  if (chunk_size < 1024)
    throw std::invalid_argument("chunk size must be at least 1024");
  std::vector<SbeMessage> messages;
  size_t offset = 0;
  while (offset < bytes.size()) {
    size_t chunk_end = offset + (chunk_size - offset % chunk_size);
    if (chunk_end > bytes.size())
      chunk_end = bytes.size();
    if (offset + kHeaderBytes > chunk_end) {
      if (offset < bytes.size() && offset + kHeaderBytes <= bytes.size() &&
          chunk_end == bytes.size())
        break;
      offset = chunk_end;
      continue;
    }
    uint16_t block_length = read_u16(bytes, offset);
    uint16_t template_id = read_u16(bytes, offset + 2);
    if (template_id == 0 && block_length == 0) {
      // Zero padding to the chunk end terminates a chunk.
      offset = chunk_end;
      continue;
    }
    DecodeResult result = decode_message(bytes, offset);
    // A message must not tear across a chunk boundary.
    if (offset + result.bytes_consumed > chunk_end && chunk_end != bytes.size()) {
      throw std::invalid_argument("message tears across chunk boundary");
    }
    messages.push_back(result.message);
    offset += result.bytes_consumed;
    if (offset >= chunk_end)
      offset = chunk_end;
  }
  return messages;
}

}  // namespace eng
