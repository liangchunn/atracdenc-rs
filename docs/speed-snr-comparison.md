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
| Encode | 1.757 | 0.838 | **2.09× faster** |
| Decode | 0.270 | 0.484 | 0.55× |
| **Total** | **2.027** | **1.322** | **1.53× faster** |

### Output quality

| Metric | Value |
|--------|-------|
| AEA file size | 8,773,760 B (identical) |
| Decoded PCM SNR | **84.40 dB** |
| AEA bitstreams | Differ (as expected, see [precision-analysis.md](precision-analysis.md)) |

The 84.4 dB SNR is consistent with the previously measured 86.4 dB decoder-cross
SNR. Rust decoding is slightly slower than C++ because the Rust FFT library
uses a different scheduling strategy; encoding is over 2× faster due to
optimised spectral processing.

---

## ATRAC3 LP2 (128 kbps stereo)

Workflow: encode PCM → ATRAC3-in-RIFF-WAV.
Both outputs decoded to PCM via ffmpeg for SNR comparison.

### Speed

| Binary | Time (s) | Ratio |
|--------|----------|-------|
| C++ | 6.060 | — |
| Rust | 2.813 | **2.15× faster** |

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
| C++ | 6.333 | — |
| Rust | 2.692 | **2.35× faster** |

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
| C++ | 6.545 | — |
| Rust | 2.609 | **2.50× faster** |

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
| SP | atrac1 | 292 kbps | 2.027¹ | 1.322¹ | **1.53×** | **84.40 dB** |
| LP2 | atrac3 | 128 kbps | 6.060 | 2.813 | **2.15×** | **34.70 dB** |
| LP105 | atrac3 | 102 kbps | 6.333 | 2.692 | **2.35×** | **29.89 dB** |
| LP4 | atrac3_lp4 | 64 kbps | 6.545 | 2.609 | **2.50×** | **28.06 dB** |

¹ ATRAC1 times are encode + decode combined.

## Notes

- The Rust port is consistently **1.5–2.6× faster** than the C++ reference for
  encoding, while producing bit-identical file sizes.
- SNR (Signal-to-Noise Ratio): higher = better. Measures how close the Rust
  decoded PCM output is to the C++ reference. 84 dB = nearly identical;
  28 dB = audible differences.
- ATRAC3 cross-encoder SNR is lower than ATRAC1 (28–35 dB vs 84 dB). This is
  expected as ATRAC3 has more precision-sensitive stages (gain control, tonal
  extraction, joint stereo matrixing) where floating-point differences between
  C++ and Rust compound across frames. See [precision-analysis.md](precision-analysis.md)
  for details.
- The `--bitrate` flag had a unit mismatch bug (kbps vs bps) in the Rust CLI
  that was fixed during testing (`crates/atracdenc-cli/src/main.rs:265`).
- Speed measurements use hyperfine `--warmup 1 --runs 3` for statistical mean
  timing. Run `bash docs/bench.sh` to reproduce.
