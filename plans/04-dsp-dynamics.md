# 04 — DSP dynamics: gain processor, transient detector, spectral upsampler, delay buffer

## Goal

Port the time-domain dynamics machinery used by ATRAC1/ATRAC3 window switching and ATRAC3 gain control: gain modulator/demodulator, transient detector + gain-curve analysis, the Planck-windowed spectral upsampler, and the generic delay buffer.

## Prerequisites

Phases 02 (util) and 03 (FFT wrapper — the upsampler needs forward/inverse real FFT at 512/4096 points).

## C++ sources

| C++ file | Lines | Rust target |
|---|---|---|
| `atracdenc/src/gain_processor.h` | 122 | `atracdenc-core/src/dsp/gain.rs` |
| `atracdenc/src/gain_processor_ut.cpp` | 4592 (mostly data) | `atracdenc-core/tests/gain_processor.rs` + data module |
| `atracdenc/src/transient_detector.{h,cpp}` | 74+484 | `atracdenc-core/src/dsp/transient.rs` |
| `atracdenc/src/transient_detector_ut.cpp` | 54 | `#[cfg(test)]` in `transient.rs` |
| `atracdenc/src/transient_spectral_upsampler.{h,cpp}` | 92+182 | `atracdenc-core/src/dsp/upsampler.rs` |
| `atracdenc/src/transient_spectral_upsampler_ut.cpp` | 297 | `#[cfg(test)]` in `upsampler.rs` |
| `atracdenc/src/delay_buffer.h` | 52 | `atracdenc-core/src/dsp/delay_buffer.rs` |

## Steps

### 1. Gain processor (`dsp/gain.rs`)

C++ `TGainProcessor<T>` is a CRTP template: `T` supplies codec constants (`GainLevel[]`, `GainInterpolation[]`, `ExponentOffset`, `GainInterpolationPosShift`, `LocScale`, `LocSz`, `MDCTSz`) and `T::SubbandInfo::TGainPoint {Level, Location}`. Only ATRAC3 instantiates it (via `TAtrac3Data`), but keep it generic.

Rust design — constants via a trait, processor generic over it:

```rust
pub struct GainPoint { pub level: u32, pub location: u32 }

pub trait GainParams {
    const GAIN_LEVEL: &'static [f32];          // filled by at3 in phase 08
    const GAIN_INTERPOLATION: &'static [f32];
    const EXPONENT_OFFSET: i32;
    const GAIN_INTERPOLATION_POS_SHIFT: i32;
    const LOC_SCALE: u32;
    const LOC_SZ: u32;
    const MDCT_SZ: usize;
}

pub struct GainProcessor<P: GainParams>(PhantomData<P>);
impl<P: GainParams> GainProcessor<P> {
    pub fn get_gain_inc(level_idx_cur: u32) -> f32;
    pub fn get_gain_inc2(level_idx_cur: u32, level_idx_next: u32) -> f32;
    // C++ returns std::function closures; in Rust return impl Fn... or
    // plain methods taking the gain points + buffers:
    pub fn demodulate(gi_now: &[GainPoint], gi_next: &[GainPoint],
                      out: &mut [f32], cur: &[f32], prev: &[f32]);
    pub fn modulate(gi_cur: &[GainPoint], buf_cur: &mut [f32], buf_next: &mut [f32]);
    // modulate is a no-op when gi_cur is empty (C++ returned empty std::function;
    // callers check before invoking — replicate by early return)
}
```

Port the two loops from `gain_processor.h` lines 57–121 verbatim (level interpolation: per-point `level *= gainInc` over `LOC_SZ` samples, trailing flat region to `MDCT_SZ/2`). Note the `incPos` computation uses next point's level or `EXPONENT_OFFSET` when last.

The unit test defines its own small test-params class — check `gain_processor_ut.cpp` for the constants the test injects and mirror that with a test-local `GainParams` impl.

### 2. Transient detector (`dsp/transient.rs`)

From `transient_detector.{h,cpp}`:

```rust
pub struct TransientDetector {
    short_sz: u16, block_sz: u16, n_short_blocks: u16,
    hpf_buffer: Vec<f32>,           // block_sz + FIR_LEN(21), 20-sample prev carry
    last_energy: f32, last_transient_pos: u16,
}
impl TransientDetector {
    pub fn new(short_sz: u16, block_sz: u16) -> Self;
    pub fn detect(&mut self, buf: &[f32]) -> bool;
    pub fn last_transient_pos(&self) -> u32;
}

pub struct GainCurvePoint { pub level: u32, pub location: u32 }
pub struct CurveBuilderCtx { pub last_level: f32, pub last_hpf_energy: f32, pub last_target: f32 }

pub fn analyze_gain(input: &[f32], max_points: u32, use_rms: bool,
                    subframe_low: Option<&mut Vec<f32>>,
                    subframe_high: Option<&mut Vec<f32>>) -> Vec<f32>;

pub fn calc_curve(input: &[f32], ctx: &mut CurveBuilderCtx,
                  next_level: Option<f32>, min_score: f32,
                  yaml_log: Option<&mut dyn std::io::Write>,   // hooks into phase 08 yaml_log
                  subframe_low: Option<&[f32]>, subframe_high: Option<&[f32]>)
                  -> Vec<GainCurvePoint>;
```

- Port the 21-tap HP FIR coefficients and `HPFilter` from `transient_detector.cpp`, plus the energy-comparison logic in `Detect`.
- `CalcCurve` (~most of the 484 lines) is the ATRAC3 gain-curve builder; port faithfully — it also writes the YAML debug log lines. Keep the yaml writer as `Option<&mut dyn Write>` so phase 08 plugs in the file.
- C++ default args (`minScore = 2.0`, `nextLevel = {}`) become explicit parameters at call sites or builder-style helpers; pick explicit params.

### 3. Spectral upsampler (`dsp/upsampler.rs`)

From `transient_spectral_upsampler.{h,cpp}` (doc comments in the header describe the algorithm precisely):

```rust
pub struct ProcessResult { pub signal: Vec<f32>, pub high_freq_ratio: f32 }

pub struct SpectralUpsampler {
    low_cut_bin: usize,
    win: Vec<f32>,                  // Planck-taper window, 512
    fwd: RealFftPlan,               // 512-pt forward real FFT
    inv: RealFftPlan,               // 4096-pt inverse real FFT
}
impl SpectralUpsampler {
    pub const IN_N: usize = 512;
    pub const UPSAMPLE: usize = 8;
    pub const OUT_N: usize = 4096;
    pub const DEFAULT_EPS: f32 = 0.15;
    pub const HIGH_FREQ_THRESHOLD: f32 = 0.05;
    pub fn new(sample_rate: f32, low_cut_hz: f32, epsilon: f32) -> Self;
    pub fn process(&self, input: &[f32]) -> ProcessResult;
}
```

- C++ uses `kiss_fftr` (real FFT). With rustfft, either use the `realfft` crate (add as dependency) or complex FFT with real packing. **Decision: add `realfft` (thin wrapper over rustfft)** — simpler and matches kiss_fftr semantics (N/2+1 bins).
- Port from `transient_spectral_upsampler.cpp`: Planck window construction, low-cut bin computation from `low_cut_hz`, the FFT-domain zero-pad upsampling with amplitude scaling, and `high_freq_ratio` energy computation (before HPF).
- Mind kiss_fftr vs realfft scaling conventions: kiss inverse real FFT is unnormalized — check what scaling `transient_spectral_upsampler.cpp` applies and adjust so the *output* matches.

### 4. Delay buffer (`dsp/delay_buffer.rs`)

Port `delay_buffer.h` `TDelayBuffer<T, N, S>` as a const-generic struct (used by ATRAC3 encoder). Read the header at port time; ~50 lines.

### 5. Tests to port

- **`gain_processor_ut.cpp`** (4592 L): mostly embedded expected-value tables for modulate/demodulate energy behavior. Port as integration test `tests/gain_processor.rs`; move the big constant tables into a `mod data` (consider `include!` of a generated `.rs` data file — generate it once from the C++ file with a small script, or transcribe directly). Use the C++ tolerance values.
- **`transient_detector_ut.cpp`** (54 L): `AnalyzeGain` cases — port verbatim.
- **`transient_spectral_upsampler_ut.cpp`** (297 L): output size, DC removal, high-freq-ratio behavior on synthetic tones. Tolerances may need slight loosening vs C++ since kissfftr→realfft changes low-order bits; keep within the same order of magnitude as the C++ epsilons.

## Acceptance criteria

- All four modules compile with no `unsafe`; all ported tests pass.
- Gain modulate→demodulate identity: new sanity test — modulating then demodulating with the same gain points reconstructs the input within 1e-4 (validates the pair before ATRAC3 uses it).
