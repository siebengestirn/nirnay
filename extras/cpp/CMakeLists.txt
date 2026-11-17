cmake_minimum_required(VERSION 3.20)
project(nirnay_extras_conformance LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release CACHE STRING "" FORCE)
endif()

# The shared exact-numeric/order-book sources use __int128 for checked
# intermediate arithmetic. That is a GCC/Clang 64-bit extension, not ISO C++.
# The support matrix is explicit and enforced here rather than assumed.
include(CheckCXXSourceCompiles)
check_cxx_source_compiles("
int main() {
  __int128 value = static_cast<__int128>(1) << 64;
  return value > 0 ? 0 : 1;
}" NIRNAY_HAS_INT128)
if(CMAKE_SIZEOF_VOID_P LESS 8)
  message(FATAL_ERROR "The extras numeric oracle requires a 64-bit target.")
endif()
if(NOT NIRNAY_HAS_INT128)
  message(FATAL_ERROR
    "The extras numeric oracle requires a compiler with __int128 support "
    "(GCC or Clang on a 64-bit target). MSVC is not a supported build host.")
endif()

# Shared fixed-point/order-book sources come from the active verification
# oracle. Only the SBE and conformance probes are retained here.
set(ORACLE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../verification/cpp-oracle/src")

# Shared sources are compiled without -Wpedantic because __int128 is a
# documented language extension. This keeps GCC and Clang on the
# same contract instead of relying on a Clang-only warning suppression.
add_library(eng_oracle STATIC
  "${ORACLE_DIR}/numeric.cpp"
  "${ORACLE_DIR}/market_state.cpp"
)
target_include_directories(eng_oracle PUBLIC "${ORACLE_DIR}")
target_compile_options(eng_oracle PRIVATE -Wall -Wextra -Werror)

# Extras-owned SBE code is ISO-clean and retains the stricter pedantic gate.
add_library(eng_sbe STATIC src/sbe.cpp)
target_include_directories(eng_sbe PUBLIC src)
target_compile_options(eng_sbe PRIVATE -Wall -Wextra -Wpedantic -Werror)
target_link_libraries(eng_sbe PUBLIC eng_oracle)

add_library(eng_numeric INTERFACE)
target_link_libraries(eng_numeric INTERFACE eng_oracle eng_sbe)

enable_testing()

# Apply line-oriented order-book operations and print canonical JSON. The Rust
# counterpart is retained under rust/conform_driver.rs for a future port to the
# active market-book crate.
add_executable(eng_conform tests/conform_driver.cpp)
target_link_libraries(eng_conform PRIVATE eng_numeric)
target_compile_options(eng_conform PRIVATE -Wall -Wextra -Wpedantic -Werror)

# Decode the synthetic SBE golden capture and print one canonical line per
# message, so a Rust decoder can be diffed against it.
add_executable(eng_sbe_dump tests/sbe_dump.cpp)
target_link_libraries(eng_sbe_dump PRIVATE eng_numeric)
target_compile_options(eng_sbe_dump PRIVATE -Wall -Wextra -Wpedantic -Werror)

add_executable(eng_sbe_test tests/sbe_roundtrip_test.cpp)
target_link_libraries(eng_sbe_test PRIVATE eng_numeric)
target_compile_options(eng_sbe_test PRIVATE -Wall -Wextra -Wpedantic -Werror)
add_test(NAME sbe_roundtrip
  COMMAND eng_sbe_test ${CMAKE_CURRENT_SOURCE_DIR}/../tests/golden/sample_replay.sbe)
