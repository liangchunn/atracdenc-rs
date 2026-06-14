# 00 — Overview: atracdenc C++ → Rust migration

## Goal

Completely migrate the `atracdenc` C++ project (`./atracdenc/`, ~26k lines) to Rust:

- **Encoders**: ATRAC1, ATRAC3 (incl. LP4 @64kbps), ATRAC3+ (GHA tonal encoding)
- **Decoder**: ATRAC1
- **Containers**: AEA, OMA, RIFF/AT3, RealMedia (.rm), RAW
- **CLI**: full option parity with the original binary (incl. `--yaml-log`)
- **Tests**: all GoogleTest unit tests and Python integration tests ported to native Rust tests

The original C++ tree at `./atracdenc/` is the reference and stays untouched.

## Locked-in decisions

| Decision | Choice | Rationale |
|---|---|---|
| Crate layout | Two crates: `atracdenc-core` (lib) + `atracdenc-cli` (bin `atracdenc`) | User requirement: CLI and core library separate; matches original single-binary project |
| FFT | `rustfft` crate | Replaces vendored kissfft; mature pure-Rust; tests are tolerance-based so bit parity with kissfft is not required |
| PCM WAV IO | `hound` crate | Replaces libsndfile / Media Foundation; pure Rust, covers the 44.1kHz/16-bit input requirement |
| CLI parsing | `clap` (derive) | Replaces getopt_long + bundled MSVC getopt |
| Verification | Tolerance-based tests + decodability, **not** bit-exact vs C++ | Bit-exactness would require porting kissfft and matching FPU rounding; out of scope |
| `--yaml-log` | **Ported** | Valuable for diffing ATRAC3 gain-control decisions between C++ and Rust builds |
| License | LGPL-2.1-or-later | Derived work of LGPL code (incl. FFmpeg-derived tables/DSP); must remain LGPL |

## Target layout

```
.                            # workspace root
├── Cargo.toml               # [workspace] members
├── crates/
│   ├── atracdenc-core/      # library crate
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── bitstream/   # MSB-first bit reader/writer + multi-pass bit-alloc framework
│   │   │   ├── dsp/         # mdct, dct4, qmf, gain processor, transient detector/upsampler
│   │   │   ├── gha/         # port of libgha (harmonic analysis + linear solver)
│   │   │   ├── atrac/       # shared: scaler/quantizer, psy helpers
│   │   │   ├── at1/         # ATRAC1 encoder + decoder
│   │   │   ├── at3/         # ATRAC3 encoder (+ yaml_log)
│   │   │   ├── at3p/        # ATRAC3+ encoder (PQF, GHA, MDCT, bitstream)
│   │   │   ├── container/   # aea, oma, at3 (riff), rm, raw
│   │   │   └── pcm/         # PCM engine (buffer, reader/writer/processor traits), wav via hound
│   │   └── tests/           # ported reference-data tests where too big for #[cfg(test)]
│   └── atracdenc-cli/       # binary crate, name = "atracdenc"
│       ├── src/main.rs
│       └── tests/           # ported integration tests (assert_cmd)
├── plans/                   # these plan files
└── atracdenc/               # original C++ reference (read-only)
```

## Phase order and dependencies

```
01 workspace setup
02 foundations (bitstream, bs_encode, util)          ── pure logic, no deps
03 dsp transforms (mdct, dct4, qmf)                  ── needs rustfft
04 dsp dynamics (gain proc, transient det/upsampler) ── needs 03 (FFT for upsampler)
05 shared atrac (scale/quant, psy)                   ── needs 02
06 pcm engine + wav io + containers                  ── needs 02 (bitstream not even required; mostly independent)
07 ATRAC1 enc+dec                                    ── needs 02,03,04,05,06
08 ATRAC3 enc (+yaml_log)                            ── needs 02,03,04,05,06
09 ATRAC3+ enc (gha, pqf, mdct, bitstream)           ── needs 02,03,05,06
10 CLI                                               ── needs 07,08,09 (can start scaffold after 06)
11 integration + validation                          ── needs 10
```

Phases 02–06 are independently testable; codecs (07–09) land one at a time with green tests throughout. After phase 07 the project is already end-to-end usable for ATRAC1.

**Current state (June 2026):** phases 01–11 are complete. ATRAC3+ encoding is
implemented (`gha/` + `at3p/` under `crates/atracdenc-core/src/`) and wired into
the facade and CLI; the CLI accepts `--advanced ghadbg=<mask>`. ATRAC3+ OMA
output decodes with ffmpeg.

**Cross-codec validation (June 2026):** Both ATRAC1 and ATRAC3 Rust encoders produce output that
the C++ reference binary can decode (and vice versa). ATRAC1 cross-decoder SNR is 86.4 dB;
ATRAC1 cross-encoder SNR is 96.6 dB. ATRAC3 OMA output decodes successfully with ffmpeg
(cross-encoder PCM SNR 64.4 dB). See `validation-notes.md` for details.

## Test strategy

- Every C++ `*_ut.cpp` becomes Rust `#[cfg(test)]` modules (or `tests/` files when reference data is large).
- Floating-point assertions use explicit tolerances mirroring the C++ `EXPECT_NEAR` values; where C++ used exact comparison on floats, choose a tight tolerance (1e-6 relative or better) since kissfft → rustfft changes low-order bits.
- Reference data files (`atracdenc/src/atrac/atrac3plus_pqf/ut/test_data/*.dat`) are consumed in place via relative paths from the workspace (no copying into the Rust tree needed; `env!("CARGO_MANIFEST_DIR")`-based paths).
- Integration tests (`atracdenc/test/integration/input_file_tests.py`) become `assert_cmd`-based Rust tests in `atracdenc-cli/tests/`.
- Final validation: encode samples with both binaries; diff `--yaml-log` output for ATRAC3 gain control; verify outputs decode (ffmpeg where available, own ATRAC1 decoder for AEA).

### C++ test → Rust test map

| C++ test | Plan | Rust location |
|---|---|---|
| `src/lib/bitstream/bitstream_ut.cpp` | 02 | `core::bitstream` tests |
| `src/lib/bs_encode/encode_ut.cpp` | 02 | `core::bitstream::encode` tests |
| `src/util_ut.cpp` | 02 | `core::util` tests |
| `src/lib/mdct/mdct_ut.cpp` | 03 | `core::dsp::mdct` tests |
| `src/gain_processor_ut.cpp` | 04 | `core/tests/gain_processor.rs` (big data tables) |
| `src/transient_detector_ut.cpp` | 04 | `core::dsp::transient` tests |
| `src/transient_spectral_upsampler_ut.cpp` | 04 | `core::dsp::upsampler` tests |
| `src/atrac/atrac_scale_ut.cpp` | 05 | `core::atrac::scale` tests |
| `src/atrac/atrac_psy_common_ut.cpp` | 05 | `core::atrac::psy` tests |
| `src/atracdenc_ut.cpp` | 07 | `core::at1` tests |
| `src/atrac3denc_ut.cpp` | 08 | `core/tests/atrac3.rs` |
| `src/atrac/atrac3plus_pqf/ut/ipqf_ut.cpp` | 09 | `core/tests/at3p_pqf.rs` (+ ref .dat) |
| `src/atrac/at3p/at3p_bitstream_ut.cpp` | 09 | `core::at3p::bitstream` tests |
| `src/atrac/at3p/at3p_gha_ut.cpp` | 09 | `core/tests/at3p_gha.rs` |
| `src/atrac/at3p/at3p_mdct_ut.cpp` | 09 | `core::at3p::mdct` tests |
| `test/integration/input_file_tests.py` | 11 | `atracdenc-cli/tests/integration.rs` |

## Not ported (and why)

| Item | Reason |
|---|---|
| Media Foundation PCM backend (`src/platform/win/pcm_io/*`) | `hound` is pure Rust, cross-platform. Note: WAV-only input vs libsndfile's AIFF/SND support — acceptable; `symphonia` is the future option if broader input matters |
| Bundled getopt (`src/platform/win/getopt/`) | `clap` |
| Windows UTF-8 shims (`utf8_file.h`, wide-argv `main` wrapper, `_wfopen`) | Rust `std::path`/`std::fs`/`std::env::args_os` are Unicode-correct natively; the UTF-8 integration tests are still ported and must pass |
| `lib/endian_tools.h` + CMake `TEST_BIG_ENDIAN` | `to_le_bytes`/`to_be_bytes` |
| FPU rounding setup (`env.cpp`, `FE_TONEAREST`) | Tolerance-based verification goal; Rust/LLVM default is round-to-nearest-even |
| x87 `__asm fistp` in `util.h::ToInt` | `f32::round_ties_even` |
| `tools/package-msys2-runtime.sh` | Rust Windows binaries are self-contained |
| `debian/` packaging | Not code; `cargo-deb` later if desired |
| `liboma` standalone tools (`omainfo`, `omacp`) | Not built by the original CMake either; out of scope |

## Progress checklist

Update this as phases complete:

- [x] 01 Workspace setup
- [x] 02 Foundations: bitstream, bit-alloc framework, util
- [x] 03 DSP transforms: MDCT/IMDCT, DCT-IV, QMF
- [x] 04 DSP dynamics: gain processor, transient detector, spectral upsampler
- [x] 05 Shared ATRAC: scaler/quantizer, psy helpers
- [x] 06 PCM engine, WAV IO, containers (AEA, OMA, RIFF/AT3, RM, RAW)
- [x] 07 ATRAC1 encoder + decoder (encode WAV→AEA, decode AEA→WAV)
- [x] 08 ATRAC3 encoder (+ yaml_log)
- [x] 09 ATRAC3+ encoder (GHA, PQF, MDCT, bitstream) — **complete**
- [x] 10 CLI (all flags; ATRAC3+ rejected with clear error)
- [x] 11 Integration tests + validation (Python suite ported, quality regression tests in place)
