# Benchmarks for 2026-06-14 13:34:50

| Binary | Version |
|--------|---------|
| PATH | atracdenc  (C++) |
| Dev | atracdenc-cli 0.1.0 |

Input: /Users/liangchun/dev/rust/atracdenc-codex/atrac1_input_from_m4a.wav

**Methodology:** SNR is computed on the decoded PCM WAV output
(not the encoded bitstream). ATRAC3/ATRAC3plus uses ffmpeg as a shared
decoder; ATRAC1 uses each binary's own decoder.

The **PATH** binary is the C++ reference; the **Dev** binary is the Rust
project build. Ratio > 1 means Dev is faster.

## ATRAC1 (SP mode, 292 kbps stereo)

### Speed

| Stage | PATH (s) | Dev (s) | Ratio |
|-------|----------|---------|-------|
| Encode | 1.524 | 0.766 | **1.98×** |
| Decode | 0.276 | 0.227 | 1.21× |
| **Total** | **1.800** | **0.993** | **1.81×** |

### Output quality

| Metric | Value |
|--------|-------|
| AEA file size | 8,773,760 B (identical) |
| Decoded PCM SNR | **81.61 dB** |

---

## ATRAC3 LP2 (128 kbps stereo)

### Speed

| Binary | Time (s) | Ratio |
|--------|----------|-------|
| PATH (C++) | 5.606 | — |
| Dev (Rust) | 2.658 | **2.10×** |

### Output quality

| Metric | Value |
|--------|-------|
| Output file size | 3,972,172 B (identical) |
| Bitrate (ffprobe) | 132296 bps (both) |
| Cross-encoder SNR | **33.25 dB** |

---

## ATRAC3 LP105 (102 kbps stereo)

### Speed

| Binary | Time (s) | Ratio |
|--------|----------|-------|
| PATH (C++) | 5.931 | — |
| Dev (Rust) | 2.645 | **2.24×** |

### Output quality

| Metric | Value |
|--------|-------|
| Output file size | 3,144,652 B (identical) |
| Bitrate (ffprobe) | 104736 bps (both) |
| Cross-encoder SNR | **29.37 dB** |

---

## ATRAC3 LP4 (64 kbps stereo, ATRAC3_LP)

### Speed

| Binary | Time (s) | Ratio |
|--------|----------|-------|
| PATH (C++) | 6.301 | — |
| Dev (Rust) | 2.372 | **2.65×** |

### Output quality

| Metric | Value |
|--------|-------|
| Output file size | 1,986,124 B (identical) |
| Bitrate (ffprobe) | 66144 bps (both) |
| Cross-encoder SNR | **27.50 dB** |

---

## ATRAC3plus (~352 kbps stereo)

### Speed

| Binary | Time (s) | Ratio |
|--------|----------|-------|
| PATH (C++) | 14.426 | — |
| Dev (Rust) | 14.955 | **0.96×** |

### Output quality

| Metric | Value |
|--------|-------|
| Output file size | 10,592,352 B (identical) |
| Bitrate (ffprobe) | 352800 bps (both) |
| Cross-encoder SNR | **41.93 dB** |

---

## Summary

| Mode | Codec | Bitrate | PATH (s) | Dev (s) | Speedup | SNR |
|------|-------|---------|----------|---------|---------|-----|
| SP | atrac1 | 292 kbps | 1.800¹ | 0.993¹ | **1.81×** | **81.61 dB** |
| LP2 | atrac3 | 128 kbps | 5.606 | 2.658 | **2.10×** | **33.25 dB** |
| LP105 | atrac3 | 102 kbps | 5.931 | 2.645 | **2.24×** | **29.37 dB** |
| LP4 | atrac3_lp4 | 64 kbps | 6.301 | 2.372 | **2.65×** | **27.50 dB** |
| — | atrac3plus | ~352 kbps | 14.426 | 14.955 | **0.96×** | **41.93 dB** |

¹ ATRAC1 times are encode + decode combined.

## Notes

- Measurements use hyperfine (--warmup 1 --runs 3) for statistical benchmarking.
- Dev (Rust) uses `RUST_LOG=off` to eliminate console I/O.
- PATH (C++) uses `>/dev/null` to suppress progress output.
- Ratio > 1 means Dev is faster than PATH.
- SNR (Signal-to-Noise Ratio): higher = better. Measures how close the Dev
  output is to the PATH (C++) reference.
- ATRAC3/ATRAC3plus: both encoded bitstreams decoded via ffmpeg for SNR.
- ATRAC1: each binary decodes its own bitstream (ffmpeg has no ATRAC1 decoder).
- C++ uses `atrac3_lp4`; Rust uses `atrac3-lp4` for LP4 mode.
