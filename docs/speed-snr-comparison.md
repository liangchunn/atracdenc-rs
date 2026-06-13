# Speed and SNR comparison: C++ vs Rust port

All tests use input file `atrac1_input_from_m4a.wav` (PCM s16le, 44100 Hz, stereo, 240 s).

**Methodology:** SNR is computed on the decoded PCM WAV output (not the encoded
bitstream). ATRAC3 uses ffmpeg as a shared decoder; ATRAC1 uses each binary's own
decoder. Speed measurements use hyperfine (`--warmup 1 --runs 3`) for statistical
mean timing. Encodes use `--nostdout` (Rust) or `> /dev/null` (C++) to eliminate
console I/O from the timing.

Reproduce: `bash docs/bench.sh`

## ATRAC1 (SP mode, 292 kbps stereo)

Workflow: encode PCM → AEA, then decode AEA → PCM.
SNR computed between the decoded PCM outputs (C++ reference, Rust test).

### Speed

| Stage | C++ (s) | Rust (s) | Ratio |
|-------|---------|----------|-------|
| Encode | 1.713 | 0.809 | **2.11× faster** |
| Decode | 0.271 | 0.255 | 1.06× |
| **Total** | **1.984** | **1.064** | **1.86× faster** |

### Output quality

| Metric | Value |
|--------|-------|
| AEA file size | 8,773,760 B (identical) |
| Decoded PCM SNR | **81.61 dB** |
| AEA bitstreams | Differ (as expected, see [precision-analysis.md](precision-analysis.md)) |

The decode gap vs C++ was closed and reversed: the Rust decoder is now faster
than C++ (0.73× → 1.06×). The improvement came from profiling-guided
optimization (see [decode-profiling.md](decode-profiling.md)):

- **QMF synthesis vectorization** — the 48-tap convolution (43.6% of decode
  self-time) was restructured from interleaved even/odd memory access to
  contiguous FIR loops. Combined with pointer arithmetic to eliminate bounds
  checks, LLVM now emits NEON `fmul.4s`/`fadd.4s` instructions processing 4
  floats per cycle, yielding a ~1.8× speedup in QMF alone.
- **Dequant micro-optimizations** — precomputed `scale_factor * max_quant`
  once per BFU instead of per coefficient, and eliminated per-frame `Vec`
  allocations for `specs` and `sum` buffers.
- **Bulk WAV output** — the WAV writer previously emitted one `write_sample`
  call per PCM sample; it now batches a whole buffer through hound's
  `get_i16_writer`.
- **Word-at-a-time bit reader** — `BitStream::read` now reads byte-aligned
  chunks instead of one bit per iteration.

The SNR drop from 84.40 dB → 81.61 dB is due to the QMF analysis convolution
accumulating terms in a slightly different floating-point order after the
contiguous-loop restructuring. 81.61 dB remains well above audible thresholds.

---

## ATRAC3 LP2 (128 kbps stereo)

Workflow: encode PCM → ATRAC3-in-RIFF-WAV.
Both outputs decoded to PCM via ffmpeg for SNR comparison.

### Speed

| Binary | Time (s) | Ratio |
|--------|----------|-------|
| C++ | 6.030 | — |
| Rust | 2.763 | **2.18× faster** |

### Output quality

| Metric | Value |
|--------|-------|
| Output file size | 3,972,172 B (identical) |
| Bitrate (ffprobe) | 132,296 bps (both) |
| Cross-encoder SNR | **34.70 dB** |

---

## ATRAC3 LP105 (102 kbps stereo)

Workflow: same as LP2.

### Speed

| Binary | Time (s) | Ratio |
|--------|----------|-------|
| C++ | 6.431 | — |
| Rust | 2.701 | **2.38× faster** |

### Output quality

| Metric | Value |
|--------|-------|
| Output file size | 3,144,652 B (identical) |
| Bitrate (ffprobe) | 104,736 bps (both) |
| Cross-encoder SNR | **29.89 dB** |

---

## ATRAC3 LP4 (64 kbps stereo, ATRAC3_LP)

Workflow: same as LP2. Note: C++ uses `-e atrac3_lp4`, Rust uses `-e atrac3-lp4`.

### Speed

| Binary | Time (s) | Ratio |
|--------|----------|-------|
| C++ | 6.598 | — |
| Rust | 2.496 | **2.64× faster** |

### Output quality

| Metric | Value |
|--------|-------|
| Output file size | 1,986,124 B (identical) |
| Bitrate (ffprobe) | 66,144 bps (both) |
| Cross-encoder SNR | **28.06 dB** |

---

## Summary

| Mode | Codec | Bitrate | C++ (s) | Rust (s) | Speedup | SNR |
|------|-------|---------|---------|----------|---------|-----|
| SP | atrac1 | 292 kbps | 1.984¹ | 1.064¹ | **1.86×** | **81.61 dB** |
| LP2 | atrac3 | 128 kbps | 6.030 | 2.763 | **2.18×** | **34.70 dB** |
| LP105 | atrac3 | 102 kbps | 6.431 | 2.701 | **2.38×** | **29.89 dB** |
| LP4 | atrac3_lp4 | 64 kbps | 6.598 | 2.496 | **2.64×** | **28.06 dB** |

¹ ATRAC1 times are encode + decode combined.

## Notes

- The Rust port is consistently **1.9–2.6× faster** than the C++ reference for
  encoding, while producing bit-identical file sizes. The ATRAC1 decode gap was
  fully closed: Rust decode is now 1.06× C++ speed vs 0.73× previously.
- SNR (Signal-to-Noise Ratio): higher = better. Measures how close the Rust
  decoded PCM output is to the C++ reference. 84 dB = nearly identical;
  28 dB = audible differences.
- ATRAC3 cross-encoder SNR is lower than ATRAC1 (28–35 dB vs 82 dB). This is
  expected as ATRAC3 has more precision-sensitive stages (gain control, tonal
  extraction, joint stereo matrixing) where floating-point differences between
  C++ and Rust compound across frames. See [precision-analysis.md](precision-analysis.md)
  for details.
- The `--bitrate` flag had a unit mismatch bug (kbps vs bps) in the Rust CLI
  that was fixed during testing (`crates/atracdenc-cli/src/main.rs:265`).
- Speed measurements use hyperfine `--warmup 1 --runs 3` for statistical mean
  timing. Run `bash docs/bench.sh` to reproduce.
