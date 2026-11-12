#include "sbe.h"

#include <fstream>
#include <iostream>

using namespace eng;

static void expect_true(bool value, const char* label) {
  if (!value) {
    std::cerr << "FAIL: " << label << "\n";
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

int main(int argc, char** argv) {
  if (argc < 2) {
    std::cerr << "usage: eng_sbe_test <golden.sbe>\n";
    return 1;
  }
  std::ifstream file(argv[1], std::ios::binary);
  std::vector<uint8_t> bytes((std::istreambuf_iterator<char>(file)),
                             std::istreambuf_iterator<char>());
  expect_true(!bytes.empty(), "golden capture non-empty");

  // Decode: challenge header + var data. The golden chunk size is 4096.
  auto messages = decode_capture(bytes, 4096);
  expect_true(messages.size() == 10, "golden decodes 10 messages");
  expect_true(messages[0].template_id == 1, "first is snapshot");
  expect_true(messages[0].symbol == "kalshi:TEST-A", "symbol decodes");
  expect_true(messages[0].bids.size() == 1, "bid count");
  expect_true(messages[0].bids[0].price == 400'000, "bid price");
  expect_true(messages[2].template_id == 2, "third is update");
  expect_true(messages[2].price_level.has_value(), "update price present");
  expect_true(messages[2].side.has_value() && *messages[2].side == 0, "update side buy");

  // Re-encode each message and decode again: canonical bytes must be exact.
  for (const auto& message : messages) {
    auto encoded = encode_message(message);
    auto again = decode_message(encoded, 0);
    expect_true(again.message == message, "encode/decode round-trips per message");
    expect_true(again.bytes_consumed == encoded.size(), "consumed equals encoded size");
  }

  // Malformed inputs must fail, not be silently accepted.
  auto truncated = bytes;
  truncated.resize(10);
  expect_throws([&] { decode_capture(truncated, 4096); }, "truncated header fails");

  std::vector<uint8_t> bad_template = bytes;
  bad_template[2] = 99;
  bad_template[3] = 0;
  expect_throws([&] { decode_capture(bad_template, 4096); }, "unknown template fails");

  // Corrupt the side byte of the first update (offset 162 + 8 + 32).
  std::vector<uint8_t> bad_side = bytes;
  bad_side[162 + 8 + 32] = 7;
  expect_throws([&] { decode_capture(bad_side, 4096); }, "invalid SideEncoding fails");

  std::vector<uint8_t> bad_schema = bytes;
  bad_schema[2] = 99;  // schemaId low byte (template patch above kept schema 1;
                       // use schema byte)
  bad_schema[4] = 99;
  expect_throws([&] { decode_capture(bad_schema, 4096); }, "unknown schema fails");

  // Invalid group dimensions: corrupt the bid group header of message 0.
  std::vector<uint8_t> bad_group = bytes;
  bad_group[24 + 1] = 0x7f;  // numInGroup high byte -> impossible count
  expect_throws([&] { decode_capture(bad_group, 4096); }, "invalid group length fails");

  // Invalid chunk size.
  expect_throws([&] { decode_capture(bytes, 512); }, "chunk size <1024 fails");

  // Message tear across an aligned chunk boundary must fail.
  // Encode one snapshot, pad so it straddles the 4096 boundary... Actually the
  // capture begins at 0, so a tear means shifting the first message by
  // chunk_size - 10 bytes of zeros, which the aligned format forbids.
  {
    std::vector<uint8_t> torn(4096 + 200, 0);
    auto one = encode_message(messages[0]);
    // Place the message so it starts 10 bytes before the boundary.
    size_t start = 4096 - 10;
    std::fill(torn.begin(), torn.begin() + start, 0);
    std::copy(one.begin(), one.end(), torn.begin() + start);
    expect_throws([&] { decode_capture(torn, 4096); }, "torn message across boundary fails");
  }

  std::cout << "all SBE round-trip checks passed (10 messages)\n";
  return 0;
}
