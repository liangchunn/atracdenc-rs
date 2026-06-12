# 03 — DSP transforms: MDCT/IMDCT, DCT-IV, QMF

## Goal

Port the frequency transforms: FFT-based MDCT/IMDCT (the heart of all three codecs), the DCT-IV wrapper, and the generic 48-tap QMF analysis/synthesis filter. The vendored kissfft is replaced by the `rustfft` crate.

## Prerequisites

Phase 02 (util). Add `rustfft` to `atracdenc-core` dependencies.

## C++ sources

| C++ file | Lines | Rust target |
|---|---|---|
| `atracdenc/src/lib/mdct/mdct.{h,cpp}` | 182+82 | `atracdenc-core/src/dsp/mdct.rs` |
| `atracdenc/src/lib/mdct/common.h` (`CalcSinCos`) | 47 | same module |
| `atracdenc/src/lib/mdct/dct.h` (`TDct4`) | 31 | `atracdenc-core/src/dsp/dct.rs` |
| `atracdenc/src/lib/mdct/mdct_ut.cpp` + `mdct_ut_common.h` | 228 | `#[cfg(test)]` in `mdct.rs` |
| `atracdenc/src/qmf/qmf.{h,cpp}` | 91+90 | `atracdenc-core/src/dsp/qmf.rs` |
| `atracdenc/src/lib/fft/kissfft_impl/*` | ~900 | **not ported** — `rustfft` |

## Key facts from the C++ code

- `TMDCTBase(n, scale)` precomputes an interleaved sin/cos twiddle table (`CalcSinCos` in `common.h` — read it for the exact formula: entries at `2π(k+1/8)/N` scaled by `sqrt(scale/N)`-style factor; port exactly) and allocates a kissfft complex FFT of size `N/4`.
- `TMDCT<N>::operator()(in: &[f32; N]) -> &[f32; N/2]`: pre-rotation (two loop halves), complex FFT of N/4 points, post-rotation writing `Buf[n]` and `Buf[n2-1-n]`. Copy the loops verbatim from `mdct.h` (lines 51–104).
- `TMIDCT<N>` (default `scale = N`, base gets `scale/2`): pre-rotation of N/2 inputs, FFT N/4, post-rotation expanding to N outputs with sign/mirror pattern (lines 115–179).
- `TDct4` wraps the same machinery for DCT-IV (used by the ATRAC3+ PQF).
- `TQmf<nIn>` (`qmf.h`): 48-tap window (`QmfWindow[48]` built from `TapHalf[24]` in `qmf.cpp` — copy the table), persistent `PcmBuffer[nIn+46]` / `PcmBufferMerge[nIn+46]` state, `Analysis(in, lower, upper)` and `Synthesis(out, lower, upper)` exactly as in the header (loops above). Also `CalcFreqResp` (used by ATRAC3 encoder for windowing decisions — check `qmf.cpp`).

## Steps

### 1. FFT abstraction

`rustfft` works on `Complex<f32>` buffers and `Arc<dyn Fft<f32>>` plans.

```rust
// dsp/fft.rs (thin wrapper so a future kissfft-port swap stays localized)
pub struct FftPlan { fft: Arc<dyn rustfft::Fft<f32>>, scratch: Vec<Complex<f32>> }
impl FftPlan {
    pub fn forward(n: usize) -> Self;
    pub fn process(&mut self, buf: &mut [Complex<f32>]);
}
```

Note: kissfft was invoked out-of-place (`FFTIn` → `FFTOut`); rustfft is in-place. The MDCT pre/post rotation loops read FFTOut with the same indices they wrote to FFTIn, so in-place is fine — just use one buffer.

### 2. MDCT module

Runtime-`n` struct instead of C++ template (codecs use N = 64, 256, 512 for AT1/AT3 and 128/256 for AT3+; tests use 32–256):

```rust
pub struct Mdct  { n: usize, sincos: Vec<f32>, fft: FftPlan, buf: Vec<f32> /* n/2 */ }
pub struct Midct { n: usize, sincos: Vec<f32>, fft: FftPlan, buf: Vec<f32> /* n   */ }

impl Mdct {
    pub fn new(n: usize, scale: f32) -> Self;          // C++ default scale = 1.0
    pub fn transform(&mut self, input: &[f32]) -> &[f32];  // input.len() == n, output n/2
}
impl Midct {
    pub fn new(n: usize, scale: f32) -> Self;          // C++ default scale = n as f32; base receives scale/2
    pub fn transform(&mut self, input: &[f32]) -> &[f32];  // input n/2, output n
}
```

- Port `CalcSinCos` from `lib/mdct/common.h` exactly (read it first; the scale factor feeds in here).
- Port the rotation loops 1:1, replacing `kiss_fft(FFTPlan, FFTIn, FFTOut)` with the wrapper call.
- Keep the C++ scale-parameter convention (`Midct::new(n, scale)` internally passes `scale/2` to the twiddle computation) so codec call sites port verbatim.

### 3. DCT-IV

Port `lib/mdct/dct.h` `TDct4` as `dsp/dct.rs::Dct4 { pub fn new(n: usize, scale: f32) -> Self; pub fn transform(&mut self, &[f32]) -> &[f32] }`. Read the header to see how it builds on TMDCTBase/FFT, port accordingly.

### 4. QMF

```rust
pub struct Qmf<const N_IN: usize> {           // or runtime n_in; const generic matches C++ template use
    pcm_buffer: Vec<f32>,        // n_in + 46
    pcm_buffer_merge: Vec<f32>,  // n_in + 46
}
impl<const N_IN: usize> Qmf<N_IN> {
    pub fn analysis(&mut self, input: &[f32], lower: &mut [f32], upper: &mut [f32]);
    pub fn synthesis(&mut self, out: &mut [f32], lower: &[f32], upper: &[f32]);
}
pub fn calc_freq_resp(sz: usize, buf: &mut [f32]) -> bool;  // from qmf.cpp
```

- Copy `TapHalf[24]` table and the window construction from `qmf.cpp`.
- Instantiations used downstream: `TQmf<512>`, `TQmf<256>` (AT1 in `atrac1_qmf.h`), and AT3's 4-band cascade (`atrac3_qmf.h`) — those wrappers are ported in phases 07/08, only the generic filter lives here.

### 5. Tests to port — `mdct_ut.cpp`

Cases (all with `mdct_ut_common.h` helpers):
- MDCT→IMDCT roundtrip at N = 32, 64, 128, 256 with random input; compare against input with the C++ tolerance (read the `EXPECT_NEAR` epsilon used; typically ~1e-4 scaled).
- Port the helper that compensates for MDCT scaling between forward/inverse.
- Add the same roundtrip at N = 512 (used by AT1/AT3 long windows) as a new test.
- New QMF test (C++ has none): analysis→synthesis of a sine through `Qmf<512>` reconstructs the delayed input within tolerance (46-sample delay) — guards the port before AT1 integration.

## Acceptance criteria

- All ported `mdct_ut` cases pass with rustfft backend.
- MDCT/IMDCT roundtrip error within the same tolerance the C++ tests used.
- QMF analysis/synthesis roundtrip test passes.
