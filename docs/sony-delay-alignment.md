# Sony decode-delay alignment for ATRAC3

This document records the investigation, reverse-engineering, and fix behind the
opt-in `--sony-delay-align` encoding mode for ATRAC3 (LP2 and LP4, stereo,
RIFF/AT3).

It is both a design note and a postmortem: it explains *why* the Rust encoder's
ATRAC3 output did not line up with Sony's reference decoder, how that was traced
all the way into Sony's `psp_at3tool.exe`, and what we ultimately shipped.

## TL;DR

- atracdenc's ATRAC3 stream, when decoded by Sony's reference decoder (e.g. a
  MiniDisc / NetMD device, or `ps?_at3tool.exe`), came out **69 samples late**
  and **clipped a few hundred samples off the tail**.
- The `+69` is the ATRAC3 **encoder delay**. We confirmed it is hard-coded as
  `0x45` (69) inside Sony's `psp_at3tool.exe` via Ghidra.
- The tail clipping had two causes: atracdenc emitted one too few frames to
  flush the codec delay, and it wrote a frame-aligned `fact` (sample count)
  instead of the true PCM length.
- All three defects are present in the upstream C++ `atracdenc` too — they are
  faithful-port behavior, not a Rust regression.
- We added an **opt-in** alignment mode that reproduces Sony's timing exactly
  (lag 0, exact length, correct `fact`) without touching the codec core, so the
  default output stays byte-identical to the C++ reference.

## Background: why timing matters

ATRAC codecs are block transforms (QMF filterbank + MDCT) and therefore have an
inherent encode/decode latency. Sony's encoder and decoder are co-designed so
their latencies cancel: a Sony-encoded file decoded by a Sony decoder reproduces
the input at **sample-exact alignment** (lag 0) and **exact length**.

A real Sony playback device (e.g. an MZ-N505 NetMD MiniDisc unit) contains the
Sony *decoder*. It assumes the stream was primed the way the Sony *encoder*
primes it. If an independent encoder primes differently, the audio plays back
slightly shifted in time. The goal of this work was to make atracdenc-encoded
ATRAC3 behave identically to Sony-encoded ATRAC3 on such hardware.

## Investigation

### 1. Measuring the offset

Reference track encoded three ways (all ATRAC3 LP2 132 kbps, 44.1 kHz stereo,
RIFF/AT3), then decoded with Sony `at3tool` and compared to the original PCM via
FFT cross-correlation + SNR:

| Encoder            | lag    | mono-mix SNR |
| ------------------ | ------ | ------------ |
| Sony `at3tool`     | **+0** | 31.7 dB      |
| C++ `atracdenc`    | +69    | 22.8 dB      |
| Rust `atracdenc`   | +69    | 22.6 dB      |

The lag was a clean, constant **+69 samples**, identical on both channels and
**identical between the C++ and Rust builds**. Repeating on a second, unrelated
track produced the same `+69`, confirming it is a structural codec delay rather
than content-dependent. The Rust port also matched the C++ reference to within
~0.1 dB SNR (and the streams differed only in late frame-data bytes), confirming
a faithful port.

Conclusion: the offset is **not a Rust bug** — it is an atracdenc-vs-Sony
difference inherited from upstream.

### 2. The tail also disappeared

Opening the decoded files revealed the Rust output was also *shorter* than
Sony's. Decomposed:

| | frames emitted | `fact` (claimed samples) | decoded length |
| --- | --- | --- | --- |
| Sony  | `ceil((S+1093)/1024)` | `S` (exact)            | `S` (exact)        |
| Rust  | `ceil(S/1024)`        | `frames*1024` (too big) | `S - (a few hundred)` |

The Sony reference decoder trims a fixed **1093 = 1024 + 69** samples from the
front of the raw decode (one frame + the encoder delay), then caps the output at
the `fact` value. So a correct stream needs:

- `frames = ceil((S + 1093) / 1024)` to flush all audio past the trim, and
- `fact = S` so the decoder emits exactly the original length.

atracdenc satisfied neither: it emitted the bare `ceil(S/1024)` frames and wrote
a frame-aligned `fact`, so real tail audio (≈ 200–500 samples depending on the
file) was clipped.

### 3. Why a naive fix does not work

The clean, content-independent way to remove a constant integer delay is to
shift the PCM. But two constraints made this subtle:

- **No chopping allowed.** Simply advancing the input by 69 samples (dropping the
  first 69) loses the leading 1.5 ms of audio — unacceptable.
- **69 is not divisible by 4.** The ATRAC3 QMF decimates by 4 (four sub-bands),
  so the 69-sample compensation cannot be expressed as an integer shift in the
  band or frame domain. It must be handled at the PCM input rate.

This is why the alignment had to be done as an explicit, opt-in framing scheme
rather than a one-line shift, and why we went into the binary to learn exactly
what Sony does.

## Reverse engineering `psp_at3tool.exe`

Tooling: `rizin` for triage, **Ghidra 12.1.2 headless** (Java post-script via the
decompiler) for analysis. The binary is a 32-bit PE (`x86:LE:32`), MSVC-built,
no PDB.

### Findings

1. **The codec is statically linked** and called through a name-resolved
   function-pointer table (`atrac_init_encode`, `atrac_set_encode_algorithm`,
   `atrac_encode`, `atrac_flush_encode`, `atrac_get_buffer_request`). No imports
   or exports expose it.

2. **The encoder delay is hard-coded as 69.** The driver computes its framing
   from a per-codec delay constant returned by `FUN_004039e0`:

   ```c
   undefined4 FUN_004039e0(int *codec) {
       undefined4 delay = 0xb8;          // 184 (other mode)
       if (*codec == 3) delay = 0x45;    // 69  -> ATRAC3
       return delay;
   }
   ```

   `0x45 = 69` — the exact value measured externally, now confirmed from inside
   Sony's binary. The driver uses `total_samples - 69` when computing its
   frame/padding layout.

3. **Sony's codec is structurally identical to atracdenc.** Following
   `atrac_encode` → `FUN_00404db0` → `FUN_00436db0` → `FUN_00436f10`:
   - The QMF window table sits at `0x490f44` and holds the *same* 48-tap
     coefficients atracdenc uses (cross-referenced by searching the binary for
     atracdenc's `TAP_HALF` float constants).
   - The QMF analysis (`FUN_00440c30`) carries a **46-sample history** frame to
     frame — matching atracdenc's `HALF_HISTORY` even+odd layout.
   - The per-frame state machine deinterleaves PCM, zero-pads to 1024, and emits
     **one priming frame** (`return (*counter != 1) + 1` — `1` = no output yet,
     `2` = frame produced), exactly like atracdenc's `lookahead_pending`.

   Because the filterbank, history size, and priming structure all match, the
   `+69` difference is **framing/alignment**, not a different transform.

4. **The driver front-pads.** `FUN_004032c0` (the encode loop) calls
   `atrac_get_buffer_request`, then arranges the partial frame at the *front* of
   the stream (right-aligning the remainder, zero-filling the head), and finishes
   with `atrac_flush_encode`. It even prints
   `"Warning: imcomplete wav file (%d sample missing)"`, i.e. it explicitly
   tracks priming/flush sample accounting.

### Reverse-engineering takeaway

The delay is exactly `69`, the decoder trim is exactly `1024 + 69 = 1093`, and
the codec internals are the same as atracdenc. The alignment is achievable
purely at the I/O/framing layer; no QMF/MDCT changes are required.

## The fix

A new opt-in setting, `At3Settings::sony_delay_align` (CLI: `--sony-delay-align`),
scoped to **ATRAC3 LP2 and LP4, stereo, RIFF/AT3**. It is **off by default**;
default output is byte-for-byte unchanged.

The 69-sample delay is the QMF cascade group delay and is therefore independent
of bitrate/frame format. It was verified against Sony `at3tool` for both LP2
(132 kbps, 384-byte frames) and LP4 (66 kbps, joint-stereo, 192-byte frames) —
both decode at `+69` unaligned and `+0` aligned. Sony's decoder only accepts the
standard MiniDisc modes (it refuses the intermediate atracdenc bitrates such as
146 kbps), so alignment is restricted to LP2/LP4 and non-LP2/LP4 bitrates are
rejected rather than silently producing un-decodable output.

When enabled, the facade (`crates/atracdenc/src/lib.rs`) does three things and
leaves the core encoder untouched:

1. **Advance by 69 samples** via an `AlignedReader` PCM adapter that feeds the
   input starting 69 samples in, so the decoded output lands at lag 0.
2. **Emit `ceil((total + 1093) / 1024)` frames**, zero-padding the tail through
   the same adapter so the decoder's 1093-sample trim never clips real audio.
3. **Write `fact = total_samples`** (the true per-channel count) instead of the
   frame-aligned default. This required a small addition to the RIFF/AT3
   container writer (`At3Output::set_fact_samples`,
   `crates/atracdenc-core/src/container/at3.rs`).

Constants live next to the code with provenance comments:

```rust
const ATRAC3_FRAME_SAMPLES: u64 = 1024;
const ATRAC3_ENCODER_DELAY: u32 = 69;          // FUN_004039e0 -> 0x45
const ATRAC3_DECODER_DELAY: u32 = 1024 + 69;   // one frame + encoder delay
```

### Verification (decoded by Sony `at3tool`)

| Input              | lag (before → after) | length (before → after)      |
| ------------------ | -------------------- | ---------------------------- |
| impulse (2 s)      | +69 → **+0**         | clipped → **exact (88 200)** |
| track A (2 min)    | +69 → **+0**         | −513 → **exact (5 600 700)** |
| track B (4 min)    | +69 → **+0**         | clipped → **exact (10 591 936)** |

Emitted frame counts now match Sony exactly (88 / 5471 / 10345). Encoder SNR is
unchanged (the alignment does not touch quantization).

### Safety / regression

- Default (non-aligned) output is byte-identical to before (`cmp`-verified) — the
  C++ parity guarantee is preserved.
- Unit tests assert the setting plumbing, the `fact`/frame-count contract for
  both LP2 and LP4, and rejection of mono / non-RIFF / non-ATRAC3 /
  non-LP2-LP4-bitrate inputs.
- Integration tests (`crates/atracdenc/tests/sony_delay_align.rs`) encode a
  committed WAV fixture (`tests/fixtures/impulse.wav`, 88 200 stereo samples) and
  assert both the aligned framing and the unchanged default framing (regression
  guard).

## Known limitation

The first **~69 samples (≈ 1.5 ms)** are reconstructed as codec-edge artifacts
rather than sample-faithful values (head error ≈ 735 RMS vs Sony's ≈ 194). This
is the direct consequence of the advance being **sub-frame and not divisible by
4**: the leading samples fall into the filterbank warm-up region. The audio
energy is present (not silence) and the bulk of the track is sample-aligned, so
the primary goal — matching Sony's *playback timing* on hardware — is met.

Closing this last 1.5 ms to bit-exact parity would require replicating Sony's
internal filterbank priming (a sub-band, non-integer compensation), which is a
deeper codec change scoped as possible future work.

## Usage

```bash
# LP2 (~132 kbps)
atracdenc -e atrac3 --container riff --sony-delay-align -i in.wav -o out.at3

# LP4 (~66 kbps, joint stereo)
atracdenc -e atrac3-lp4 --container riff --sony-delay-align -i in.wav -o out.at3
```

Input must be 44.1 kHz / 16-bit / **stereo** WAV. The flag is rejected for mono
input, non-RIFF containers, non-ATRAC3 codecs, and bitrates that do not resolve
to LP2 or LP4.

## Reproducing the analysis

- Encoder cross-check uses Sony's `ps?_at3tool.exe` under CrossOver/Wine
  (`at3rs/third_party/ATRAC-Codec-TOOL/`), which encodes/decodes real ATRAC3.
- SNR/lag are measured by decoding `.at3` to WAV and cross-correlating against
  the original PCM (`docs/compute_snr.py` for plain SNR; the alignment work used
  an FFT-correlation + SNR-refinement variant).
- The binary findings were produced with Ghidra headless
  (`analyzeHeadless <proj> -import psp_at3tool.exe`) plus a small decompiler
  post-script; key addresses: `FUN_004039e0` (delay constant), `FUN_004032c0`
  (encode driver), `FUN_00436db0`/`FUN_00436f10` (codec core), `0x490f44` (QMF
  window table).
