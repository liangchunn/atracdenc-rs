# 07 — ATRAC1 encoder + decoder

## Goal

Port the complete ATRAC1 codec: encoder (WAV→AEA/RAW) and decoder (AEA→WAV). First end-to-end usable codec; proves the whole stack (QMF → MDCT → scale/quant → bit alloc → bitstream → container).

## Prerequisites

Phases 02–06.

## C++ sources

| C++ file | Lines | Rust target |
|---|---|---|
| `atracdenc/src/atrac/at1/atrac1.{h,cpp}` | 137+45 | `atracdenc-core/src/at1/data.rs` |
| `atracdenc/src/atrac/at1/atrac1_qmf.h` | header-only | `atracdenc-core/src/at1/qmf.rs` |
| `atracdenc/src/atrac/at1/atrac1_bitalloc.{h,cpp}` | 64+355 | `atracdenc-core/src/at1/bitalloc.rs` |
| `atracdenc/src/atrac/at1/atrac1_dequantiser.{h,cpp}` | 32+71 | `atracdenc-core/src/at1/dequantiser.rs` |
| `atracdenc/src/atrac1denc.{h,cpp}` | 126+247 | `atracdenc-core/src/at1/{mdct.rs, encoder.rs, decoder.rs}` |
| `atracdenc/src/atracdenc_ut.cpp` | 116 | `#[cfg(test)]` in `at1/mdct.rs` |

## Codec structure (from the C++)

- 3 QMF bands: low/mid (0–5.5k/5.5–11k, 128 samples each) + hi (11–22k, 256 samples), via two cascaded `TQmf` stages (`atrac1_qmf.h`: `Atrac1AnalysisFilterBank` = `TQmf<512>` then `TQmf<256>`, with a 39-sample delay-compensation buffer on the hi band; `Atrac1SynthesisFilterBank` mirror).
- Per band: MDCT with long (512/256) or short (64) windows, chosen by per-band transient detection (detectors: low/mid `(16,128)`, hi `(16,256)`).
- MDCT scaling constants (from `atrac1denc.h`): `Mdct512(1), Mdct256(0.5), Mdct64(0.5), Midct512(1024), Midct256(512), Midct64(128)`.
- 512 spectral coefficients → up to 52 BFUs; scaler (phase 05) + bit allocation (`TAtrac1SimpleBitAlloc` with `TBitsBooster`, loudness/ATH weighting) → 212-byte frames per channel.
- `TAtrac1Data`: BFU layout tables (`SpecsPerBlock`, `SpecsStartLong/Short`, `BfuAmountTab`, scale table `ScaleTable[64]`, sine window), `TBlockSizeMod` (per-band log MDCT size).
- `TAtrac1EncodeSettings`: `bfuIdxConst` (0=auto, 1..8), `fastBfuNumSearch`, window mode + `windowMask` (from `--notransient`).

## Steps

### 1. `at1/data.rs` — constants and settings

Port `TAtrac1Data` tables from `atrac1.{h,cpp}` as `const`/`static` arrays: `SpecsPerBlock[52]`, `SpecsStartLong[52]`, `SpecsStartShort[52]`, `BfuAmountTab[8]`, `ScaleTable[64]` (generated or literal — check the cpp; if computed at init, compute in a `LazyLock` or `const fn`), sine window for MDCT overlap, `NumSamples=512`, frame size 212. Port `TBlockSizeMod` as:

```rust
pub struct BlockSizeMod { pub log_count: [u32; 3] }   // check actual C++ fields
impl BlockSizeMod { pub fn new(low_short: bool, mid_short: bool, hi_short: bool) -> Self; }
```

Port `TAtrac1EncodeSettings` as `at1::EncodeSettings { bfu_idx_const: u32, fast_bfu_num_search: bool, window_mode: WindowMode, window_mask: u32 }` with the same defaults as the C++ ctor.

Provide the `BlockLayout` (phase 05) construction from these tables for `Scaler::scale_frame`.

### 2. `at1/qmf.rs` — 3-band filter bank

Port `atrac1_qmf.h`:

```rust
pub struct Atrac1AnalysisFilterBank  { qmf1: Qmf<512>, qmf2: Qmf<256>, delay_buf: ..., /* 39-sample hi-band delay */ }
impl Atrac1AnalysisFilterBank { pub fn analysis(&mut self, pcm: &[f32], low: &mut [f32], mid: &mut [f32], hi: &mut [f32]); }
pub struct Atrac1SynthesisFilterBank { ... }
impl Atrac1SynthesisFilterBank { pub fn synthesis(&mut self, pcm: &mut [f32], low: &[f32], mid: &[f32], hi: &[f32]); }
```

Read the header for the exact delay handling (hi band delayed to align with the doubly-filtered low/mid path).

### 3. `at1/mdct.rs` — windowing + MDCT for the 3 bands

Port `TAtrac1MDCT::Mdct/IMdct` from `atrac1denc.cpp`: per-band long/short window logic with sine-window overlap-add, the 16-sample overlap tails kept in `PcmBuf{Low,Mid,Hi}` (sizes 256+16, 256+16, 512+16), spectrum inversion of odd bands (`InvertSpectr` from util). Struct owns the 6 MDCT/IMDCT instances with the constants listed above.

### 4. `at1/bitalloc.rs`

Port `atrac1_bitalloc.{h,cpp}`:
- `IAtrac1BitAlloc` → trait or just the concrete `Atrac1SimpleBitAlloc`.
- `TBitsBooster` (spends leftover bits on more mantissa precision).
- Bit-allocation iteration: per-BFU word lengths from scale-factor spread + ATH + loudness curve, target 212 bytes/channel; uses `BitStream` to write the frame: header (BFU amount idx, block size mods), per-BFU word lengths, scale factor indices, mantissas.
- The exact frame layout lives in this file's `WriteBitStream` — port verbatim; it must interop with the dequantiser and real MD hardware.

### 5. `at1/dequantiser.rs` (decoder side)

Port `atrac1_dequantiser.{h,cpp}`: reads a 212-byte frame from `BitStream`, reconstructs 512 spectra (block mode, word lengths, scale factors, mantissas → floats via scale table).

### 6. `at1/encoder.rs` and `at1/decoder.rs`

Port `TAtrac1Encoder`/`TAtrac1Decoder` from `atrac1denc.cpp` as `Processor` impls (phase 06 trait):
- Encoder: per-channel QMF analysis → transient detect per band → window mode (respect `EncodeSettings` overrides) → MDCT → `Scaler::scale_frame` → loudness tracking (`LoudFactor 0.006` init, psy `track_loudness`) → bit alloc → `CompressedOutput::write_frame`.
- Decoder: `CompressedInput::read_frame` → dequantise → IMDCT → QMF synthesis → PCM clamp (C++ tracks `PcmValueMax/Min`, prints clipping warnings? read the cpp) → output buffer.
- Both handle 1 or 2 channels and the engine's LOOK_AHEAD frame-delay semantics exactly as the C++ lambdas do (read `GetLambda` bodies carefully — there's a 1-frame look-ahead/delay due to MDCT overlap).

### 7. Tests

- **Port `atracdenc_ut.cpp`** (116 L): ATRAC1 MDCT long/short window encode→decode roundtrip (sine input, SNR-style tolerance). Lives as `#[cfg(test)]` in `at1/mdct.rs`.
- **New: full-codec roundtrip test** (`atracdenc-core/tests/at1_roundtrip.rs`): generate 1s of 44.1kHz mixed sines, encode to an in-memory/tmp AEA, decode back, assert correlation/SNR above a threshold (calibrate threshold by running the same material through the C++ binary once; document the measured value in the test).
- **Cross-validation (manual, documented in test comments)**: a Rust-encoded AEA should decode with the C++ `atracdenc -d` and vice versa — done in phase 11.

## Acceptance criteria

- All ported + new tests green.
- Encoder output frame size exactly 212 bytes/channel; AEA produced is accepted by the C++ decoder (manual check now, automated in phase 11).
- Decoder consumes C++-produced AEA files and yields PCM with sensible audio (SNR check vs C++ decoder output on the same file, tolerance-based).
