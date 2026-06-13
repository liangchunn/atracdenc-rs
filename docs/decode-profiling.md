# ATRAC1 decode profiling: where the time goes

This documents a sampling-profiler investigation into why the Rust ATRAC1
decoder, after earlier optimization work, is still ~1.3× slower than the C++
reference, and refutes the earlier hypothesis that FFT trait-object dispatch was
responsible.

## TL;DR

ATRAC1 decode time is dominated by **QMF synthesis (43.6% of self-time)**, a
scalar 48-tap convolution that does not autovectorize well on aarch64. The FFT
is **not** a bottleneck: the actual `rustfft` NEON kernels account for only
~2.3% of self-time, and there is no measurable trait-object dispatch overhead.
The single highest-value optimization is to vectorize `Qmf::synthesis`.

## Background

Earlier work (see [speed-snr-comparison.md](speed-snr-comparison.md)) cut decode
time from 0.484 s to 0.373 s with three changes (bulk WAV writes, a
word-at-a-time bit reader, and making the QMF window a compile-time constant),
narrowing the C++ gap from 0.55× to 0.73×. An open question remained: was the
residual gap caused by `rustfft`'s `Arc<dyn Fft<f32>>` trait-object dispatch, as
hypothesized? This profiling run answers that.

## Methodology

- **Profiler:** [`samply`](https://github.com/mstange/samply) 0.13.1 (macOS
  sampling API; no `sudo`/SIP issues, unlike `dtrace`/`cargo flamegraph`).
- **Build:** dedicated `[profile.profiling]` in the workspace `Cargo.toml`
  (`inherits = "release"`, `debug = true`, `strip = false`) — full optimization
  plus DWARF symbols for symbolication.
- **Workload:** decode the 240 s stereo fixture AEA (8.77 MB) 40 times via
  samply's `--iteration-count 40`, sampled at 4000 Hz. This yields ~59 k
  samples, enough for stable statistics despite each decode being only ~0.37 s.
- **Analysis:** self-time (leaf-frame) aggregation across all threads, with
  symbols resolved from samply's `--unstable-presymbolicate` sidecar.

Reproduce:

```bash
cargo install samply
cargo build --profile profiling
samply record -r 4000 --iteration-count 40 --unstable-presymbolicate -s -n \
  -o /tmp/at1.profile.json.gz -- \
  target/profiling/atracdenc -d -i in.aea -o out.wav --nostdout
samply load /tmp/at1.profile.json.gz   # opens the Firefox Profiler UI
```

## Results

Self-time breakdown (59,010 weighted samples, 40 iterations at 4000 Hz):

| % self | Function | Stage |
|-------:|----------|-------|
| **43.57%** | `dsp::qmf::Qmf::synthesis` | QMF synthesis |
| **13.78%** | `at1::dequantiser::Atrac1Dequantiser::dequant` | dequantise |
| 10.23% | `write` (syscall) | WAV output I/O |
| **8.50%** | `dsp::mdct::Midct::transform` | IMDCT pre/post-twiddle |
| 5.30% | `read` (syscall) | AEA input I/O |
| 3.97% | `Atrac1Decoder::process_frame` | per-frame glue |
| 1.73% | `at1::mdct::Atrac1Mdct::imdct` | IMDCT block driver |
| 1.51% | `_platform_memset` | buffer clears |
| 1.51% | `_platform_memmove` | ring-buffer copies |
| 1.42% | `WavWriter::write` | sample encode loop |
| ~2.3% | `rustfft::neon::*` (radix4, butterflies, bit-reverse transpose) | **actual FFT** |
| ~2.5% | `szone`/`tiny`/`small` malloc & free | allocations |

## Conclusions

### The FFT trait-object hypothesis is refuted

The real FFT work — `rustfft::neon::neon_radix4::*`, `NeonF32Butterfly16`,
`array_utils::bitreversed_transpose`, `validate_and_iter_unroll2x` — sums to only
**~2.3%** of decode self-time, and `rustfft` is using **NEON SIMD** kernels.
`process_with_scratch` (the dispatch entry point) registers at 0.05%. There is no
vtable/dispatch hotspot. Optimizing or de-virtualizing the FFT would be
essentially invisible in the total.

### QMF synthesis is the dominant cost (43.6%)

`Qmf::synthesis` (`crates/atracdenc-core/src/dsp/qmf.rs`) runs a 48-tap dot
product per output sample, twice per frame (the 512- and 256-tap filter-bank
stages). Even with the window now a compile-time constant, the inner loop is
scalar and is not autovectorizing well on aarch64 — likely because the even/odd
`s1`/`s2` split with `.step_by(2)` interleaving prevents the compiler from
emitting clean contiguous NEON FMAs. This is nearly half of all decode time and
is the clear optimization target.

### Secondary costs

- **`dequant` (13.8%)** — per-coefficient `BitStream::read` plus the
  scale/quant arithmetic in `Atrac1Dequantiser::dequant`.
- **`write` syscall (10.2%)** — genuine kernel I/O for the output WAV. The
  user-space encode loop (`WavWriter::write`) is already batched at 1.4%; the
  10.2% is the kernel write itself and is hard to reduce further.
- **`Midct::transform` (8.5%)** — the MDCT pre/post-twiddle butterflies that
  surround the FFT call; this is the scalar arithmetic in `mdct.rs`, not the FFT.

## Recommended next step

Vectorize `Qmf::synthesis` (and, for symmetry, `Qmf::analysis`): restructure the
dot product so the compiler emits NEON FMA over contiguous spans, or hand-write
it with `core::arch::aarch64` / `std::simd`. Because QMF synthesis is 43.6% of
decode, even a 2× speedup there would cut total decode time by ~20% and likely
push the Rust decoder past the C++ reference.
