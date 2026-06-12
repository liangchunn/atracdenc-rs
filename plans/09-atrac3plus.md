# 09 — ATRAC3+ encoder: libgha port, PQF, MDCT, GHA tonal encoding, bitstream

## Goal

Port the ATRAC3+ encoder (WAV → OMA/RIFF/RAW), the largest phase: the GHA library (C), the 16-band analysis PQF (C, FFmpeg-derived), the per-subband MDCT, GHA-based tonal encoding, and the VLC bitstream writer with its FFmpeg-derived tables.

## Prerequisites

Phases 02–06 (bitstream + bs_encode, MDCT/DCT-IV, scale, containers). Independent of 07/08.

## C++/C sources

| Source | Lines | Rust target |
|---|---|---|
| `atracdenc/src/lib/libgha/` (`src/gha.c`, `src/sle.c`, `include/libgha.h`) — submodule, branch `at3pghadev` | ~670 | `atracdenc-core/src/gha/mod.rs` (+ `sle.rs`) |
| `atracdenc/src/atrac/atrac3plus_pqf/atrac3plus_pqf.{c,h}` + `atrac3plus_pqf_data.h` | 147+38+132 | `atracdenc-core/src/at3p/pqf.rs` |
| `atracdenc/src/atrac/at3p/ff/atrac3plusdsp.c` + `atrac3plus.h` + `atrac3plus_data.h` | 204+165+1672 | `atracdenc-core/src/at3p/ff_tables.rs` (+ ipqf in `pqf.rs` tests) |
| `atracdenc/src/atrac/at3p/at3p_tables.{h,cpp}` | 81+130 | `atracdenc-core/src/at3p/tables.rs` |
| `atracdenc/src/atrac/at3p/at3p_mdct.{h,cpp}` | 94+154 | `atracdenc-core/src/at3p/mdct.rs` |
| `atracdenc/src/atrac/at3p/at3p_gha.{h,cpp}` | 79+867 | `atracdenc-core/src/at3p/gha.rs` |
| `atracdenc/src/atrac/at3p/at3p_bitstream.{h,cpp}` + `_impl.h` | 61+728+155 | `atracdenc-core/src/at3p/bitstream.rs` |
| `atracdenc/src/atrac/at3p/at3p.cpp` + `src/atrac3p.h` | 258+55 | `atracdenc-core/src/at3p/encoder.rs` |

Tests:

| C++ test | Rust target |
|---|---|
| `atrac3plus_pqf/ut/ipqf_ut.cpp` (308) + `ut/atrac3plusdsp.{c,h}` ref + `ut/test_data/*.dat` | `atracdenc-core/tests/at3p_pqf.rs` |
| `at3p/at3p_bitstream_ut.cpp` (139) | `#[cfg(test)]` in `at3p/bitstream.rs` |
| `at3p/at3p_gha_ut.cpp` (787) | `atracdenc-core/tests/at3p_gha.rs` |
| `at3p/at3p_mdct_ut.cpp` (120) | `#[cfg(test)]` in `at3p/mdct.rs` |

## Steps

### 1. `gha` module — port libgha

Check out the submodule first if absent (`git -C atracdenc submodule update --init`). Sources: `gha.c` (GHA core: FFT-based coarse frequency pick, then iterative refinement extracting amplitude/phase via least squares), `sle.c` (small dense linear-equation solver), `include/libgha.h` (API: `gha_create_ctx`, `gha_analyze_one`, `gha_analyze_all`, `gha_adjust_info`, `struct gha_info { frequency, phase, magnitude }`).

Rust design:

```rust
pub struct GhaInfo { pub frequency: f32, pub phase: f32, pub magnitude: f32 }
pub struct GhaCtx { /* size, fft plan (reuse dsp::fft), window buffers */ }
impl GhaCtx {
    pub fn new(size: usize) -> Self;
    pub fn analyze_one(&mut self, pcm: &[f32]) -> GhaInfo;
    pub fn analyze_all(&mut self, pcm: &[f32], out: &mut [GhaInfo]) -> usize;
    pub fn adjust_info(&mut self, pcm: &[f32], info: &mut [GhaInfo], ...);  // mirror exact C API incl. resuidal/error returns
}
```

- libgha is built with atracdenc's kissfft injected (`GHA_FFT_LIB`); in Rust use the phase 03 FFT wrapper.
- `sle.c` → `gha/sle.rs`: straightforward Gaussian-elimination style solver; port verbatim with f64/f32 exactly as C uses (check `FLOAT` typedef — libgha may use double internally; match it, it matters for tone extraction stability).
- Port libgha's own test vectors if cheap (its `test/` uses fctx + DTMF PCM files) — optional; the at3p_gha_ut coverage downstream is the real guard. Mark as stretch.

### 2. `at3p/ff_tables.rs` — FFmpeg-derived constants

Transcribe from `ff/atrac3plus_data.h` (1672 L) and `ff/atrac3plusdsp.c`: IPQF coefficients, sine table/Hann window init (C does runtime init of `sine_table`/`hann_window` — in Rust use `LazyLock<[f32; N]>` or precompute `const` via build script; LazyLock is simpler), amp scalefactor tables. Mechanical but large: prefer a one-off conversion script (`sed`/python) from the C header into Rust `pub static` arrays; keep FFmpeg attribution + LGPL header in the file.

### 3. `at3p/pqf.rs` — 16-band analysis PQF

Port `atrac3plus_pqf.c`: ring-buffer state, the `atrac3plus_pqf_data.h` coefficient tables, and the DCT-IV (phase 03 `Dct4`) callback it uses internally (C code takes an MDCT callback — in Rust just call `Dct4` directly). API:

```rust
pub struct At3pPqf { /* state per channel */ }
impl At3pPqf {
    pub fn new() -> Self;
    pub fn analyze(&mut self, input: &[f32; 2048], out: &mut [f32; 2048]); // check exact C signature (frame = 2048 samples → 16 sb × 128)
    pub fn frame_size(&self) -> usize;  // mirror C constants from atrac3plus_pqf.h
}
```

For the test, also port the **reference IPQF** from `ut/atrac3plusdsp.c` (FFmpeg synthesis filter) into the test crate only (`tests/common/ipqf_ref.rs`) — it's test scaffolding, not product code.

### 4. `at3p/tables.rs`

Port `at3p_tables.{h,cpp}`: `THuffTables` (Huffman/VLC code tables for tones etc.), scale tables, inverse mantissa table. Verify against `at3p_bitstream_ut` expectations.

### 5. `at3p/mdct.rs`

Port `at3p_mdct.{h,cpp}`: `TAt3pMDCT`/`TAt3pMIDCT` — per-subband MDCT-256 with per-subband window selection (SINE/STEEP from `ff` Hann/steep windows), overlap state per channel/subband. Port `at3p_mdct_ut.cpp` (sine/steep window combination roundtrips) as module tests.

### 6. `at3p/gha.rs` — tonal analysis

Port `at3p_gha.{h,cpp}` (867 L): `TAt3PGhaData` (wave params: `FreqIndex/AmpSf/AmpIndex/PhaseIndex`; per-subband wave info + envelope; `ToneSharing[16]`, `SecondIsLeader`, sentinel values `EMPTY_POINT = u32::MAX`, `INIT = u32::MAX-1`), and `MakeGhaProcessor0(stereo)`:

```rust
pub trait GhaProcessor {
    // b1/b2: two consecutive subband buffers per channel; w1/w2: residual outputs
    fn do_analyze(&mut self, b1: [&[f32]; 2], b2: [&[f32]; 2],
                  w1: &mut [f32], w2: &mut [f32]) -> Option<&At3pGhaData>;
}
pub fn make_gha_processor0(stereo: bool) -> Box<dyn GhaProcessor>;
```

Internals: per-subband GHA extraction loops (uses `gha` module), amplitude/phase/frequency quantization to indices, frame-to-frame tone tracking and envelope continuation, channel leader/sharing decisions. This is the subtlest file in the project — port it function-by-function with `at3p_gha_ut.cpp` cases enabled incrementally (mono single tone → stereo → partial frames).

### 7. `at3p/bitstream.rs`

Port `at3p_bitstream.{h,cpp}` + `_impl.h`: `CreateFreqBitPack` (tone frequency packing asc/desc), wave-parameter VLC encoding via `tables.rs`, subband info, channel unit header, integration with phase 02 `bs_encode` for bit budgeting. Port `at3p_bitstream_ut.cpp` (freq bit-pack asc/desc order cases) as module tests.

### 8. `at3p/encoder.rs`

Port `at3p.cpp` `TAt3PEnc::TImpl` + the public `atrac3p.h` API:

```rust
pub struct At3pSettings { pub use_gha: u8 }   // bitflags GHA_PASS_INPUT | GHA_WRITE_TONAL | GHA_WRITE_RESIDUAL
impl Default for At3pSettings { ... }          // GHA_ENABLED
pub fn parse_advanced_opt(opt: &str, settings: &mut At3pSettings);  // for CLI --advanced
pub struct At3pEncoder { ... }                 // Processor impl; NUM_SAMPLES = 2048
```

Pipeline per frame (2048 samples/channel): PQF analysis → per-subband MDCT → GHA tone extraction (subject to `use_gha` flags) → residual MDCT spectra + tone params → bitstream → `CompressedOutput` (OMA frame size for AT3+ per phase 06 params).

### 9. Tests

- **`tests/at3p_pqf.rs`** ← `ipqf_ut.cpp`: DC test, sequence test, energy test of PQF→(reference IPQF) chain against `atracdenc/src/atrac/atrac3plus_pqf/ut/test_data/{ipqftest_pcm_mr.dat, ipqftest_pcm_out.dat}` — reference the files in place via a path constant relative to `CARGO_MANIFEST_DIR` (`../../atracdenc/src/atrac/atrac3plus_pqf/ut/test_data/`). Keep the C++ tolerances.
- **`tests/at3p_gha.rs`** ← `at3p_gha_ut.cpp` (787 L): synthetic sine extraction mono/stereo, partial frames, tracked tones across frames. Numeric sensitivity warning: results depend on the FFT and f64 vs f32 in libgha — if exact index expectations fail, first verify the gha module against libgha's own DTMF tests before loosening anything.
- **module tests** in `bitstream.rs`, `mdct.rs` as above.
- New smoke test: encode 0.5s stereo sine to OMA(AT3+); assert frame count/size; (ffmpeg decode check lands in phase 11).

## Risks / notes

- This phase is ~6.5k lines of source incl. 1.8k of tables; expect it to take as long as phases 02–08 combined.
- Suggested internal order: ff_tables → pqf (+test) → tables → mdct (+test) → gha lib → at3p gha (+test) → bitstream (+test) → encoder.
- The C PQF/IPQF and libgha use `float`/`double` mixes — match each declaration's precision exactly when porting; do not "upgrade" floats to f64.

## Acceptance criteria

- All four ported test suites green, incl. PQF reference-data comparisons within original tolerances.
- AT3+ OMA output decodes with ffmpeg (manual; automated in phase 11).
- `--advanced` flag parsing matches C++ (`ParseAdvancedOpt` accepted tokens — read the cpp for the exact grammar).
