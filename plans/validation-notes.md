# Validation Notes

## 2026-06-12 ATRAC1 Parity Pass

Rust-side checks completed:

- `cargo test -p atracdenc-core --test at1_roundtrip -- --nocapture`
  - Passes.
  - Adds a mono ATRAC1 AEA encode/decode quality regression over deterministic multitone PCM.
  - Current Rust roundtrip measures roughly 10 dB after delay/gain alignment; the test uses an 8 dB regression floor until C++ calibration is available.

C++ reference build attempt:

- `cmake -S atracdenc -B atracdenc/build`
  - Configured successfully.
  - GTest was not found, so C++ unit tests were skipped by CMake.
- `cmake --build atracdenc/build --target atracdenc -j 4`
  - Failed at final link.
  - The linker selected x86_64 objects while `/opt/homebrew/lib/libsndfile.dylib` is arm64-only.
- `cmake -S atracdenc -B atracdenc/build-arm64 -DCMAKE_OSX_ARCHITECTURES=arm64`
  - Configured successfully.
- `cmake --build atracdenc/build-arm64 --target atracdenc -j 4`
  - Failed with the same final-link architecture mismatch.
  - `file atracdenc/build-arm64/src/CMakeFiles/pcm_io.dir/pcm_io_sndfile.cpp.o` reports a universal object containing both x86_64 and arm64 slices.
  - `file /opt/homebrew/lib/libsndfile.dylib` reports arm64 only.

Blocked parity items:

- Rust-encoded AEA decoded by C++ reference binary.
- C++-encoded AEA decoded by Rust binary.
- C++-calibrated ATRAC1 SNR threshold.
- C++ gtest run for `atracdenc_ut.cpp`.

Next unblock options:

- Build/link the C++ reference against an x86_64 `libsndfile`, or force an arm64-only final link with the local CMake/toolchain.
- Install GTest for the active architecture if C++ unit parity is required locally.
