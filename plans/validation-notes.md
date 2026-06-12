# Validation Notes

## 2026-06-13 Cross-codec validation

Using the C++ reference binary at `$HOME/.local/bin/atracdenc` and the Rust build at
`target/release/atracdenc`.

### ATRAC1 cross-decode

Test input: 3.0 s mono 44100 Hz WAV (440 Hz + 1 kHz + 5 kHz tones with fade).

All four encode/decode paths produce output:

| Path | SNR vs original |
|---|---|
| Rust enc → Rust dec | -3.5 dB |
| Rust enc → C++ dec  | -3.5 dB |
| C++ enc → C++ dec   | -3.5 dB |
| C++ enc → Rust dec  | -3.5 dB |

Cross-decoder comparison (same AEA, different decoder):

| Comparison | SNR |
|---|---|
| Rust AEA: C++ dec vs Rust dec | 86.4 dB |
| C++ AEA:  C++ dec vs Rust dec | 86.4 dB |

Cross-encoder comparison (same decoder, different AEA source):

| Comparison | SNR |
|---|---|
| C++ dec:  C++ enc vs Rust enc | 96.6 dB |
| Rust dec: C++ enc vs Rust enc | 96.6 dB |

**Conclusion:** ATRAC1 encoders and decoders are interoperable between C++ and Rust.
Cross-decoder output is nearly identical (86+ dB). Cross-encoder bitstreams produce
nearly identical PCM when decoded by the same decoder (96+ dB).

### ATRAC3 cross-encode

Test input: same as above.

Both Rust and C++ OMA outputs decode successfully with ffmpeg:

| Comparison | SNR |
|---|---|
| C++ OMA ffmpeg-decoded vs original | -3.0 dB |
| Rust OMA ffmpeg-decoded vs original | -3.0 dB |
| C++ vs Rust decoded PCM | 64.4 dB |

File sizes: C++ 50400 bytes, Rust 50016 bytes (0.76% difference).

**Conclusion:** ATRAC3 encoders produce ffmpeg-decodable output with comparable quality.

## 2026-06-12 ATRAC1 Parity Pass (superseded)

Rust-side checks completed:

- `cargo test -p atracdenc-core --test at1_roundtrip -- --nocapture`
  - Passes.
  - Mono ATRAC1 AEA encode/decode quality regression over deterministic multitone PCM.
  - Current Rust roundtrip measures roughly 10 dB after delay/gain alignment; uses an 8 dB regression floor.

C++ reference build attempt (from source) was blocked by arm64/x86_64 architecture mismatch with
`libsndfile`. This is superseded by the cross-codec validation above using the pre-built C++ binary.
