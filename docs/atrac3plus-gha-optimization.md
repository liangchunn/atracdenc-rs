# ATRAC3plus encode optimization — GHA tonal analysis

**Date:** 2026-06-14
**Scope:** `atracdenc-core` — `gha/mod.rs`, `bitstream/mod.rs`, `at3p/encoder.rs`
**Outcome:** ATRAC3plus encode ~7% faster, flipped from slower-than-C++ to
faster-than-C++, with **byte-identical** output (SNR unchanged).

---

## 1. Problem statement

The cross-encoder benchmark (`docs/speed-snr-comparison.md`) showed every codec
except ATRAC3plus beating the C++ reference comfortably:

| Mode | Codec | C++ (s) | Rust (s) | Ratio |
|------|-------|---------|----------|-------|
| SP | atrac1 | 1.800 | 0.993 | 1.81× |
| LP2 | atrac3 | 5.606 | 2.658 | 2.10× |
| LP105 | atrac3 | 5.931 | 2.645 | 2.24× |
| LP4 | atrac3_lp4 | 6.301 | 2.372 | 2.65× |
| — | **atrac3plus** | **14.426** | **14.955** | **0.96× (slower)** |

ATRAC3plus being the lone regression — and only marginally slower (~3.6%) — was
the anomaly to explain and fix, ideally in line with the 2–2.6× wins elsewhere.

---

## 2. Investigation

### 2.1 Two prior hypotheses (allocation churn)

Two earlier analyses both concluded the slowdown was **heap-allocation churn**:
the C reference uses `alloca()` for the GHA Newton scratch buffers
(`libgha/src/gha.c:268-282`) and preallocated context buffers, whereas the Rust
port allocated fresh `Vec`s per call in `adjust_info_newton_md`, `analyze_one`,
`real_fft_forward`, `resample_fft`, plus per-frame `.to_vec()` copies and
per-frame bitstream encoder recreation.

### 2.2 Profiling (the decisive step)

Rather than act on the hypothesis, the encoder was profiled with `samply`
(profiling build, 2 kHz sampling) on `atrac1_input_from_m4a.wav`, and self-time
was resolved against the binary's symbols. Result:

| Function | Self-time | Lib |
|---|---|---|
| `gha::GhaCtx::adjust_info_newton_md` | **57.6%** | atracdenc |
| `__sincosf_stret` | 13.1% | libsystem_m |
| `gha::search_omega_newton` | 7.5% | atracdenc |
| `at3p::bitstream::QuantUnitsEncoder::…` | 5.7% | atracdenc |
| `gha::sle::sle_solve` | 2.7% | atracdenc |
| `bitstream::BitStream::write` | 1.1% | atracdenc |
| `_platform_memset` / `_platform_memmove` | 0.6% / 0.6% | libsystem_platform |
| `_nanov2_free` (malloc/free) | **0.2%** | libsystem_malloc |

**This refuted the allocation hypothesis.** Allocation/free was ~0.2%, and
memset/memmove together ~1.2%. The cost was overwhelmingly **CPU-bound
arithmetic** in the GHA Generalized Harmonic Analysis (tonal extraction):
`adjust_info_newton_md` (the multidimensional Newton refinement) at 57.6% plus
its `sin`/`cos` at ~13%.

This also explains why only ATRAC3plus regressed: ATRAC1/ATRAC3 do not run GHA,
so they never hit this hot loop. The Newton refinement runs on the order of
`max_loops (7) × tones × subbands (8) × channels` per frame, over thousands of
frames.

### 2.3 Constraint

The encoded bitstream must stay **byte-identical** (the project cross-validates
at 41.93 dB SNR vs the C++/ffmpeg path, and the GHA code is intentionally
precision-matched to the reference, mixing `f32`/`f64` exactly). So any change
had to preserve the exact floating-point results. Every change below was
verified by `cmp` against a pre-change reference encode (`/tmp/base.oma`,
SHA-256 captured) after each step.

---

## 3. Root-cause insight: dead work in the normal-equation matrix

`adjust_info_newton_md` builds a `3·dim × 3·dim` normal-equation matrix `M`
(blocks for amplitude / frequency / phase) from per-sample basis vectors, then
solves it. Reading the code carefully (and confirming against the C reference)
revealed that the matrix is assembled as an **upper block-triangle** and then a
symmetrization step copies the lower block-triangle over the upper one:

```text
m[r0 + j + dim*1] = m[r1 + j + dim*0];   // block(0,1) <- block(1,0)
m[r0 + j + dim*2] = m[r2 + j + dim*0];   // block(0,2) <- block(2,0)
m[r1 + j + dim*2] = m[r2 + j + dim*1];   // block(1,2) <- block(2,1)
```

The lower-triangle blocks `(1,0)`, `(2,0)`, `(2,1)` are **never accumulated** —
the assembly only ever writes blocks `(0,0),(0,1),(0,2),(1,1),(1,2),(2,2)`.
After the per-iteration `M`-clear they are `0`, and nothing writes them. Hence
the symmetrization **unconditionally overwrites the upper off-diagonal blocks
with zero**.

Consequences (all verified, the existing code even had a comment noting `baw`
was dead):

- The accumulations into the off-diagonal blocks — `a01`, `a02`, `a12` — are
  **dead**: they are computed every sample and then zeroed.
- The per-sample basis arrays that feed *only* those dead entries — `baw`,
  `bap`, `bwp` — are **entirely dead**.
- The final matrix is effectively **block-diagonal** (blocks `(0,0)`, `(1,1)`,
  `(2,2)` plus the RHS column); the surviving terms are `a00`, `a11`, `a22`.

Removing the dead computations changes neither the final matrix nor the solve,
so the output is bit-for-bit identical — but it eliminates a large fraction of
the inner-loop multiply-adds (especially for the common `dim == 1` single-tone
case, where it removes 3 of 6 matrix dot-products and 3 of 8 per-sample basis
terms).

---

## 4. Fixes

All changes are output-identical and were verified by `cmp` after each step.

### 4.1 Dead-code elimination in `adjust_info_newton_md` (the main win)

`crates/atracdenc-core/src/gha/mod.rs`

- **Matrix assembly:** removed the accumulations into the off-diagonal blocks
  (`a01`, `a02`, `a12`), their `*= 2.0` scaling, and the now-redundant
  symmetrization copies. The entries remain `0` from the per-iteration clear,
  matching the reference output exactly. Only `a00`/`a11`/`a22` are computed.
- **Per-sample loop:** removed the `baw`, `bap`, `bwp` basis arrays (and their
  scratch buffers), since they fed only the dead matrix entries. `bww`/`bpp`
  (which feed the live diagonal blocks) are retained.

> Bit-exactness note: the surviving accumulators sum the same values in the same
> order as before; removing independent dead accumulators interleaved between
> them does not perturb the live sums.

### 4.2 Scratch-buffer reuse (Newton + FFT)

`crates/atracdenc-core/src/gha/mod.rs`

- Moved the Newton scratch (`M`, `fx0`, `ba`, `bw`, `bp`, `bww`, `bpp`) onto
  `GhaCtx`, grown on demand and reused across calls — the Rust analog of the C
  reference's `alloca` + preallocated context. Every element is overwritten
  before it is read each iteration, so reuse is safe.
- `real_fft_forward` / `resample_fft` now reuse preallocated complex/real
  scratch on `GhaCtx` instead of allocating a `Vec` per call.
- `analyze_one` no longer clones the working buffer.

Although profiling showed allocation was a small fraction, removing the
per-call allocations (hundreds per frame) measurably reduced memset/memmove and
allocator traffic and contributed the final ~2.6% of the speedup.

### 4.3 `BitStream::write` — byte-chunked writes

`crates/atracdenc-core/src/bitstream/mod.rs`

Rewrote the bit-by-bit write loop (one `divmod` + masked OR per bit) into a
byte-aligned, word-at-a-time loop mirroring the existing `read`. Produces
byte-identical output; shared by all codecs (covered by existing bitstream
round-trip tests).

### 4.4 Removed per-frame buffer copies

`crates/atracdenc-core/src/at3p/encoder.rs`

`encode_frame` previously made up to four `.to_vec()` copies of the
2048-sample PQF band buffers per frame to satisfy the borrow checker before
calling `do_analyze`. Restructured to pass the buffers by reference using
disjoint field borrows (`buf` is read, `prev_buf` is written — different fields
of `ChannelCtx`). Pure data-flow change; cannot affect output.

---

## 5. A gotcha worth recording: codegen-induced FP drift

An initial attempt also applied **bounds-check elision (fixed-length
subslices) and loop-invariant hoisting to the per-sample loop**. This silently
broke byte-identity (first differing byte at the same offset across variants),
even though the arithmetic was textually equivalent. Bisection isolated it to
that loop: the restructuring changed LLVM codegen (almost certainly
floating-point contraction / instruction selection around the `sin`/`cos`
dependent computations), and the GHA Newton trajectory is chaotic enough that a
sub-ULP per-iteration difference compounds across the encode into different tone
selections and a different bitstream.

Findings:

- **Safe** (verified byte-identical): subslice bounds-elision in the *matrix*
  loop; scratch reuse; FFT-buffer reuse; the dead-code removal; the `.to_vec()`
  removal; the `BitStream::write` rewrite.
- **Unsafe** (reverted): any restructuring of the *per-sample* basis loop. Its
  arithmetic is now kept verbatim from the reference.

Lesson: for precision-locked ports, verify byte-identity after *every* change
rather than batching, and treat the trig-bearing inner loop as untouchable.

---

## 6. Results

Clean, separate `hyperfine` runs (8 runs, 2 warmups) on the same machine,
input `atrac1_input_from_m4a.wav`, container `oma`:

| Binary | Time (s) | σ |
|--------|----------|---|
| C++ reference (PATH) | 14.457 | ±0.353 |
| Rust, before | ~14.96 | — |
| **Rust, optimized** | **13.643** | ±0.069 |

- **~7% faster** than the pre-optimization Rust build.
- ATRAC3plus flipped from **0.96× → ~1.06×** vs C++ (now faster, in line with
  the other codecs).
- Output is **byte-identical** to the pre-optimization bitstream
  (10,592,352 B); cross-encoder SNR unchanged at **41.93 dB**.
- All test suites pass (`cargo test`); `cargo fmt` / `cargo clippy` clean.

> Caveat on measurement: an *interleaved* hyperfine run (C++ and Rust
> alternating) showed high variance and occasionally favored C++ on wall-clock,
> while Rust consistently used *less* user CPU. Separate runs on a quiet system
> give the stable figures above. Benchmark each binary separately.

---

## 7. Not done (and why)

- **Persisting the per-frame bitstream encoder** (`At3PBitStream::write_frame`
  recreates `BitStream` + boxed part-encoders each frame). Blocked by
  `SpecFrame<'a>`'s borrow lifetime threading through
  `dyn BitStreamPartEncoder<SpecFrame<'a>>`; persisting it requires an invasive
  HRTB / owning-`SpecFrame` refactor for ~1% upside. Deferred.
- **Reducing `QuantUnitsEncoder` per-quant-unit allocations** (`mant`, `tmp`,
  `res`, nested `data`). The 5.7% there is mostly VLC compute, not allocation;
  low ROI relative to refactor risk. Candidate for future work if needed.
- **Faster trig / a lighter FFT** for the small 128/256 transforms. The trig
  result is part of the byte-exact contract, so it cannot change without
  breaking parity; not pursued.

---

## 8. Reproduction

Profile (profiling build keeps debug symbols):

```bash
cargo build --profile profiling -p atracdenc-cli
samply record --save-only --unstable-presymbolicate -o /tmp/at3p.profile.json.gz -r 2000 -- \
  target/profiling/atracdenc -e atrac3plus -i atrac1_input_from_m4a.wav -o /tmp/prof.oma
```

Verify byte-identity after a change:

```bash
cargo build --release -p atracdenc-cli
RUST_LOG=off target/release/atracdenc -e atrac3plus -i atrac1_input_from_m4a.wav -o /tmp/new.oma
cmp /tmp/base.oma /tmp/new.oma   # must report no difference
```

Benchmark (separately, not interleaved):

```bash
hyperfine --warmup 2 --runs 8 'atracdenc -e atrac3plus -i atrac1_input_from_m4a.wav -o /tmp/cpp.oma --container oma'
hyperfine --warmup 2 --runs 8 'env RUST_LOG=off target/release/atracdenc -e atrac3plus -i atrac1_input_from_m4a.wav -o /tmp/dev.oma --container oma'
```
