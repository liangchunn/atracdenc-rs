# 08 — ATRAC3 encoder (+ yaml_log)

## Goal

Port the ATRAC3 encoder (WAV → OMA/RIFF/RM/RAW), including gain control with transient detection/spectral upsampling, tonal component extraction, joint stereo, and the `--yaml-log` gain-control debug logger.

## Prerequisites

Phases 02–06 (esp. 04: gain processor, transient detector, upsampler; 06: OMA/AT3/RM containers).

## C++ sources

| C++ file | Lines | Rust target |
|---|---|---|
| `atracdenc/src/atrac/at3/atrac3.{h,cpp}` | 280+56 | `atracdenc-core/src/at3/data.rs` |
| `atracdenc/src/atrac/at3/atrac3_qmf.h` | 44 | `atracdenc-core/src/at3/qmf.rs` |
| `atracdenc/src/atrac/at3/atrac3_bitstream.{h,cpp}` | 71+803 | `atracdenc-core/src/at3/bitstream.rs` |
| `atracdenc/src/atrac3denc.{h,cpp}` | 135+869 | `atracdenc-core/src/at3/{mdct.rs, encoder.rs}` |
| `atracdenc/src/yaml_log.h` | 93 | `atracdenc-core/src/at3/yaml_log.rs` |
| `atracdenc/src/atrac3denc_ut.cpp` | 1144 | `atracdenc-core/tests/atrac3.rs` |

## Codec structure (from the C++)

- 4 QMF bands × 256 samples (`Atrac3AnalysisFilterBank` in `atrac3_qmf.h`: 3 cascaded `TQmf` + delay compensation).
- Per band: gain control (modulation before MDCT, per `SubbandInfo` gain points) then MDCT-512 (scale 1) with the ATRAC3 window; odd-band spectrum inversion.
- `TAtrac3Data` (port whole): `GainLevel[16]`, `GainInterpolation[31]`, `ExponentOffset/GainInterpolationPosShift/LocScale/LocSz/MDCTSz` (→ implements phase 04 `GainParams`), scale table, BFU layout (`SpecsPerBlock`, `BlocksPerBand`, `SpecsStartLong/Short`, `MaxBfus=32`, `NumQMF=4`), `TContainerParams` table (bitrate ↔ frame size ↔ joint stereo: 66/105/132 kbps + bitrate variants for RM), `SubbandInfo` (gain points per band, max 8), `TTonalComponents`/`TTonalBlock`.
- Encoder pipeline (`atrac3denc.cpp`): per-channel 4-band analysis → look-ahead buffering (`LookAheadBuf[2][4][640]` = prev_128 | cur_256 | lookahead_256) → `TSpectralUpsampler` (sample_rate 11025 per band, low-cut; read ctor args in cpp) → `CalcCurve` gain points (`CurveBuilderCtx` per ch/band) → gain modulation → MDCT → peak/overflow limiting (`PrevPeak`, `LimitRel`, `RelationToIdx`) → tonal component extraction (`ExtractTonalComponents` / `MapTonalComponents` with spectral-flatness gating) → optional joint-stereo `Matrixing()` → scale → bitstream writer.
- `TAtrac3BitStreamWriter` (`atrac3_bitstream.cpp`, 803 L — the big one): per-SCE (single channel element) encoding: subband info (gain points), tonal components (VLC), word lengths/scale factors (multiple coding modes incl. VLC), mantissa packing; bit-budget allocation using the phase 02 `bs_encode` framework; joint-stereo frame layout for 2 channels.
- `TAtrac3EncoderSettings`: container params ref, `noGainControlling`, `noTonalComponents`, `bfuIdxConst`, source channels.

## Steps

### 1. `at3/data.rs`

Port all `TAtrac3Data` constants/tables from `atrac3.{h,cpp}`. Implement `GainParams` (phase 04 trait) for an `At3GainParams` marker type. Port `TContainerParams` as:

```rust
pub struct ContainerParams { pub bitrate: u32, pub frame_sz: u16, pub joint_stereo: bool }
pub fn params_for_bitrate(bitrate: u32) -> Option<ContainerParams>;   // RM --bitrate path
pub const LP2: ContainerParams = ...;  // 132kbps, the named presets used by CLI (atrac3 / atrac3_lp4)
pub const LP4: ContainerParams = ...;  // 66kbps joint stereo
```

(Read `atrac3.h` for the exact preset table and lookup helpers used by `main.cpp`.)

Port `SubbandInfo` with `add_subband_curve`/gain-point storage (read the header; the UT exercises it directly).

### 2. `at3/qmf.rs`

Port `Atrac3AnalysisFilterBank` (4-band cascade with delay buffers) from `atrac3_qmf.h` on top of `dsp::qmf::Qmf`. No synthesis needed (no AT3 decoder).

### 3. `at3/yaml_log.rs`

Port `yaml_log.h` semantics onto `std::io::Write`:

```rust
pub struct YamlLog<W: Write> { out: W }
impl<W: Write> YamlLog<W> {
    pub fn write_float_seq(&mut self, v: &[f32], precision: usize);
    // document/frame/band emit helpers as needed by encoder.rs and dsp::transient::calc_curve
}
```

Field names and formatting must match the C++ output exactly (fixed-point, default precision 4, same key names/nesting per the example in `yaml_log.h` header comment) — phase 11 diffs Rust vs C++ logs. The log is threaded into `calc_curve` (phase 04 already takes `Option<&mut dyn Write>`) and the encoder's per-frame document writer.

### 4. `at3/mdct.rs`

Port `TAtrac3MDCT` from `atrac3denc.{h,cpp}`: `mdct(specs[1024], bands[4], max_levels, gain_modulators)`, `midct(...)` (used by UT), `CalcGainEnergyScale`, `MakeGainModulatorArray`, plus the windowing with `prev_128` overlap from the delay buffer. Also port free fn `RelationToIdx` (header, lines 44–52) into this module or `at3/encoder.rs`.

### 5. `at3/bitstream.rs`

Port `atrac3_bitstream.{h,cpp}` (largest single file): `TSingleChannelElement` (subband info + scaled blocks + tonal blocks), the part-encoder pipeline over phase 02's `BitStreamEncoder`, VLC tables for word-length/scale-factor deltas, tonal component encoding, mantissa packing, joint-stereo bit splitting between channels, frame padding to container frame size. Port incrementally with the UT cases as guard.

### 6. `at3/encoder.rs`

Port `TAtrac3Encoder`: settings struct, the full `GetLambda` pipeline described above, loudness tracking (same `LoudFactor 0.006`), `LookAheadPending` 1-frame delay semantics, joint stereo matrixing, yaml-log frame documents (`frame`, `time`, per-channel/band records incl. `high_freq_ratio`, `gain` RMS array, `curve_raw`/`curve_final`, `gain_energy_scale` — mirror the C++ emit points exactly).

Constructor takes `Box<dyn CompressedOutput>` + settings + optional yaml `Box<dyn Write>`.

### 7. Tests — port `atrac3denc_ut.cpp` (1144 L) to `tests/atrac3.rs`

The UT uses `ATRAC_UT_PUBLIC` to reach private members — in Rust expose the needed items as `pub(crate)` + a `#[doc(hidden)] pub mod testing` re-export, or make the test a unit test inside the module (preferred: keep big data in `tests/` but add `pub` test-only accessors behind `#[cfg(feature = "ut")]`; simplest robust choice: `pub` methods, documented as internal).

Cases to port:
- MDCT/MIDCT roundtrips with gain modulation (zero signal, sine, DC) at various gain-point configurations.
- Gain-control point placement tests (`CreateSubbandInfo` behavior on synthetic transients).
- `RelationToIdx` cases.
- Tolerances: keep C++ `EXPECT_NEAR` epsilons.

New test: full-frame encode smoke — encode 0.5s sine at LP2 into OMA in tmpdir; assert frame sizes match `ContainerParams.frame_sz` and stream is the expected length.

## Acceptance criteria

- All ported UT cases green.
- Encoded OMA/AT3 files decode with ffmpeg (`ffmpeg -i out.oma out.wav` succeeds and yields plausible audio) — manual here, automated in phase 11.
- `--yaml-log` output schema identical to C++ (same keys, nesting, precision); numeric values close (exact match not required due to FFT differences, but gain decisions — integer levels/locations — should match on typical material; investigate any divergence).
