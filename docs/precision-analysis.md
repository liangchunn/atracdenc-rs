# Floating-point precision analysis: C++ vs Rust

This documents why the C++ and Rust ATRAC1 encoders produce slightly different
bitstreams (96.6 dB cross-encoder SNR) despite implementing identical
algorithms.

## TL;DR

The first 10 frames are byte-identical. Divergence starts at frame 11 when
the loudness IIR filter has accumulated enough floating-point error to cross
a bit-allocation boundary. Only 19 of 259 frames differ (24 bytes out of
~55k), producing a 96.6 dB cross-encoder SNR. There are no implementation
bugs — the precision mismatches are expected when porting between languages
with different default intermediate precisions.

## Root cause: intermediate precision

C and Rust disagree on what "intermediate" float expressions should be. C++
tends to promote to `double` implicitly; Rust keeps everything in `f32` unless
explicitly widened. These sub-ULP differences cascade through the pipeline and
accumulate until a decision threshold is crossed.

## Precision mismatches

### 1. MDCT sin/cos table — affects every frame

The MDCT twiddle table is computed once and used for every frame. Slightly
different sin/cos values produce slightly different spectral coefficients.

| | Computation |
|---|---|
| C++ `mdct.cpp:25-36` | `cosf(omiga * i + alpha)` — argument and trig in f32 |
| Rust `mdct.rs:7-19` | `cos(omega * i as f64 + alpha) as f32` — argument and trig in f64 |

Rust's version has ~1 ULP better precision, but "better" doesn't match C++.

### 2. ScaleTable — changes scale-factor selection

ATRAC1 selects a scale factor index from a table computed as `2^(i/3 - 21)`.

| | Computation |
|---|---|
| C++ `atrac1.h:125` | `pow(2.0, double(i / 3.0 - 21.0))` — double intermediate |
| Rust `data.rs:33` | `2.0_f32.powf(i as f32 / 3.0 - 21.0)` — f32 only |

A different scale factor index changes the bit allocation for that band.

### 3. SineWindow — small per-frame noise

| | Computation |
|---|---|
| C++ `atrac1.h:130` | `sin((i + 0.5) * (M_PI / 64.0))` — double sin, stored to float |
| Rust `data.rs:41` | `((i as f32 + 0.5) * (PI / 64.0)).sin()` — f32 math |

### 4. ATH table — changes threshold decisions

| | Computation |
|---|---|
| C++ `atrac1_bitalloc.cpp:116` | `pow(10, 0.1 * x)` — double, cast to float |
| Rust `bitalloc.rs:203` | `10.0_f32.powf(0.1 * x)` — f32 only |

### 5. Loudness IIR — the amplifier

The loudness tracker is a simple IIR: `prev = 0.98 * prev + 0.02 * current`.

Both implementations compute this in f32, but their inputs (spectral energy)
already differ from the MDCT table differences above. The IIR accumulates
these tiny errors frame over frame. By frame 11, the accumulated difference
is large enough that a bit-allocation threshold crosses, and the encoder
makes a different decision about which spectral bands to quantize.

## The divergence chain

```
MDCT twiddle (1 ULP) → spectral coefficients differ (sub-ULP)
    → loudness IIR accumulates over frames (growing error)
        → frame 11: bit-allocation threshold crossed
            → 19 of 259 frames differ (24 bytes)
                → 96.6 dB PCM SNR
```

## Not the FFT

Both kiss_fft (C++) and rustfft (Rust) use `f32` arithmetic for the sizes
used (all powers of 2). Both are mixed-radix Cooley-Tukey implementations.
The FFT output is identical within float epsilon and is not a meaningful
source of divergence.

## Cross-codec validation summary

### ATRAC1

| Comparison | SNR |
|---|---|
| Encoder cross (same decoder) | 96.6 dB |
| Decoder cross (same AEA) | 86.4 dB |

### ATRAC3

| Comparison | SNR |
|---|---|
| Encoder cross (ffmpeg decoded) | 64.4 dB |

ATRAC3 has more precision-sensitive stages (gain control, tonal extraction,
joint stereo matrixing) so the cross-encoder gap is larger.
