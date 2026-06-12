# 11 — Integration tests and final validation

## Goal

Port the Python integration suite to Rust integration tests in `atracdenc-cli/tests/`, add end-to-end audio-quality checks, and run a final parity audit against the C++ binary.

## Prerequisites

Phase 10 (full CLI). Dev-dependencies for `atracdenc-cli`: `assert_cmd`, `predicates`, `tempfile`. (Optionally `hound` as dev-dep for WAV fixture generation/inspection.)

## C++/Python sources

| Source | Rust target |
|---|---|
| `atracdenc/test/integration/input_file_tests.py` (250 L, 10 CTest cases) | `atracdenc-cli/tests/integration.rs` |
| `.github/workflows/cmake.yml` (test driving) | optional `/.github/workflows/rust.yml` |

## Part 1 — Port the integration suite

Shared helpers (mirror the Python):
- `write_wav(path, samples)` — mono, 16-bit, 44100 Hz, `samples` zero frames (use `hound`).
- `UTF8_STEM = "é-入力-тест"`.
- Run the binary via `assert_cmd::Command::cargo_bin("atracdenc")` with `--nostdout`.

Port all 10 cases as `#[test]` fns (each in its own tempdir via `tempfile`):

1. **missing-input**: `-e atrac3` with nonexistent input → nonzero exit; combined stdout+stderr (lowercased) must contain `"unable to open input file"`, must NOT contain `"unsupported sample rate"`, and must include the input filename. ⚠ This pins the CLI's error wording (phase 10) — keep messages in sync.
2. **utf8-input**: atrac3 encode from UTF-8-named WAV (2048 samples) → success + non-empty `.oma`.
3. **utf8-input-atrac1**: atrac1 encode from UTF-8-named WAV (8192 samples) → success + non-empty `.aea`.
4. **utf8-output-oma** / 5. **utf8-output-at3** / 6. **utf8-output-rm**: atrac3 encode to UTF-8-named output with each suffix → success + non-empty output (exercises extension-based container inference).
7. **utf8-output-aea**: atrac1 encode to UTF-8-named `.aea`.
8. **utf8-decode-input**: encode atrac1 fixture → decode from UTF-8-named `.aea` → non-empty WAV.
9. **utf8-decode-output**: decode to UTF-8-named `.wav`.
10. **explicit-container**:
    - atrac3 + `--container riff` to a `.oma`-named file → output starts with `b"RIFF"` (container overrides extension).
    - atrac1 + `--container raw` to `.aea`-named file → success.
    - atrac1 + `--container oma` → fails, message contains `"container oma is not supported for atrac1"` (lowercased).
    - atrac3plus + `--container rm` → fails, message contains `"container rm is not supported for atrac3plus"`.

## Part 2 — Audio-quality end-to-end tests (new)

In `atracdenc-cli/tests/quality.rs` (or core `tests/`):

1. **ATRAC1 roundtrip SNR**: generate 1–2s of deterministic multi-tone WAV → encode (atrac1/AEA) → decode with our own decoder → compute SNR vs (suitably delayed) original. Calibrate the threshold once against the C++ binary on identical input and assert ≥ (cpp_snr − margin). Document the measured C++ value in a comment.
2. **Container sanity**: encoded `.oma` (AT3 + AT3+), `.at3`, `.rm` headers parse correctly: magic bytes, frame sizes, patched length fields consistent with frame count (pure-Rust header readers in the tests).
3. **ffmpeg cross-check (optional, env-gated)**: if `ffmpeg` is on PATH (or `ATRACDENC_FFMPEG=1`), decode the AT3/AT3+/AEA outputs with ffmpeg and assert nonzero, full-length PCM with finite SNR vs input. Use `#[ignore]`-by-default or runtime skip so CI without ffmpeg stays green.

## Part 3 — Parity audit vs C++ (manual, scripted)

Add `plans/validation-notes.md` (created during this phase) recording results of:

1. Build C++ reference: `cmake -B atracdenc/build atracdenc && cmake --build atracdenc/build` (needs libsndfile + gtest via brew on this machine; submodule `git -C atracdenc submodule update --init`).
2. **Cross-decode**: Rust-encoded AEA decoded by C++ `atracdenc -d`, and C++-encoded AEA decoded by Rust `-d`; both must produce clean audio (listen + SNR script).
3. **yaml-log diff**: encode the same WAV with both binaries using `-e atrac3 --yaml-log`; diff the YAML structurally (Python: parse both, compare integer fields — gain `curve_final` levels/locations — exactly; float fields within tolerance). Investigate any integer-decision divergence: acceptable only if traced to FFT low-order differences flipping a borderline decision on synthetic edge content; real-material divergence rate should be ~0.
4. **ffmpeg ABX/spot-listen** on music samples for all three codecs.
5. Run the C++ gtest suite once (`ctest`) to confirm the reference itself is green on this machine, so tolerance calibrations are trustworthy.

## Part 4 — Wrap-up

- Update `plans/00-overview.md` checklist; mark all phases done.
- Root `README.md`: usage, build, codec/container matrix, differences vs C++ (WAV-only input, no MF backend), license.
- Optional: `.github/workflows/rust.yml` — `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check` on linux/macos/windows.
- `cargo clippy` and `cargo fmt` clean across the workspace.

## Acceptance criteria

- All 10 ported integration cases + quality tests green via `cargo test`.
- Cross-decode (Part 3.2) verified both directions for ATRAC1.
- yaml-log structural diff shows matching gain-control decisions on the validation samples.
- ffmpeg successfully decodes Rust-encoded AT3/AT3+/AEA outputs.
- Workspace passes `cargo test`, `cargo clippy`, `cargo fmt --check`.
