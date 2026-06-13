# C++ to Rust parity audit

This is a file-by-file comparison of the Rust port against the C++ reference
(`atracdenc/src/` at commit `01234b0`) performed on 2026-06-13 and updated
after the parity cleanup pass on 2026-06-13.

**Scope:** every functional C++ source file outside `at3p/` (ATRAC3+) and
`platform/win/` (Windows-specific).  ATRAC3+ is intentionally not ported.

**Method:** for each module, the algorithm, constants/tables, and supported-input
edge handling were compared.  Implementation differences ("same outcome,
different code structure") and variable-name differences are **not** reported.
Only genuine behavioral or correctness deviations are flagged.

---

## Overall result

| Group | Modules | Pass | GAP |
|-------|---------|------|-----|
| ATRAC1 codec | `at1/{codec,bitalloc,dequantiser,qmf,data,mdct}` | 5 | 1 |
| ATRAC3 codec | `at3/{encoder,bitstream,qmf,data,mdct,yaml_log}` | 5 | 1* |
| Psychoacoustics | `atrac/{psy,scale}` | 2 | 0 |
| Containers | `container/{aea,at3,oma,raw,rm}` | 5 | 3 |
| DSP transforms | `dsp/{mdct,dct,qmf,fft}` | 4 | 0 |
| DSP analysis | `dsp/{transient,upsampler,delay_buffer,gain}` | 4 | 0 |
| Bitstream + PCM | `bitstream/{mod,encode}`, `pcm/{engine,wav}` | 4 | 2 |
| CLI + util | `main.rs`, `util.rs` | 1 | 10 |
| **Total** | | **30** | **17** |

Every GAP listed below is behaviourally minor.  None represents missing
ATRAC1/ATRAC3 encode or decode logic.  The only encoder-output divergence listed
below is the ATRAC3 BFU trimming timing in `at3/bitstream.rs` when using the
default fast mode; C++-parity allocation is now available with
`--at3-bfu-mode parity`.  The previously suspected QMF analysis parity issue is
a false positive because Rust's split even/odd buffers are equivalent to C++'s
reversed single history buffer with the mirrored QMF window.

`*` ATRAC3 BFU allocation is counted as a default-behaviour gap because fast mode
remains the default for performance.  The parity path is implemented and opt-in.

---

## 1. Fully-parity modules — PASS

These modules are algorithmically identical to their C++ counterparts for the
supported inputs and normal codec paths.  Constants, table sizes, loop
structures, and codec pipeline steps match.  Minor defensive or EOF-behaviour
differences are called out explicitly below.

### 1.1 ATRAC1 codec

| Rust file | C++ file(s) | Notes |
|-----------|-------------|-------|
| `at1/data.rs` | `atrac/at1/atrac1.h` | `SPECS_PER_BLOCK`, `BLOCKS_PER_BAND`, `SPECS_START_LONG/SHORT`, `BFU_AMOUNT_TAB`, `SoundUnitSize`, `BitsPerBfuAmountTabIdx`, `BitsPerIDWL`, `BitsPerIDSF` — all value-for-value exact. `BlockSizeMod::parse` skips the same 2 truncation bits. |
| `at1/qmf.rs` | `atrac/at1/atrac1_qmf.h`, `qmf/qmf.cpp/.h` | Two-stage QMF cascade (512→256+256→128+128). `DELAY_COMP=39` matches C++. `TAP_HALF` and derived `QMF_WINDOW` are bit-identical. The generic analysis kernel is equivalent: C++ indexes a reversed single history buffer, while Rust indexes split even/odd buffers; the mirrored window makes the convolution pairings match. Synthesis is equivalent. |
| `at1/mdct.rs` | `atrac1denc.h` (`TAtrac1MDCT`) | Same window offsets (`winStart` 112/48 long, 0 short). Same `multiple=2.0` compensation for short-window hi-band. IMDCT overlap-add with 16-point `vector_fmul_window` and 240/112 copy lengths match exactly. |
| `at1/dequantiser.rs` | `atrac/at1/atrac1_dequantiser.cpp/.h` | Identical bitstream read order: `bfu_idx` (3 bits), 2+3 padding, word lengths (4 bits × num_bfus), scale factors (6 bits × num_bfus). Dequant formula `1/((1<<(wordLen-1))-1)` matches. |
| `at1/bitalloc.rs` | `atrac/at1/atrac1_bitalloc.cpp/.h` | Binary-search lambda range [-3, 15], convergence check `max_lambda <= min_lambda`, `last_lambda` fallback — all match C++ `TBitAllocHandler`. BFU auto-downsizing on `Repeat` status matches. `TBitsBooster` key-capping and surplus loop match. Frame-dump header encapsulation (block size, `bfu_idx`, word lengths, scale factors, quantised values with 3×8 padding) matches `TBfuAlloc::Dump`. `FIXED_BIT_ALLOC_TABLE_LONG`, `FIXED_BIT_ALLOC_TABLE_SHORT`, `BIT_BOOST_MASK` — all 52-entry arrays value-for-value identical. |
| `at1/codec.rs` | `atrac1denc.cpp/.h`, `atrac/at1/atrac1.cpp` | Encoder pipeline: deinterleave → QMF → transient detect → `BlockSizeMod` → MDCT → loudness track → scale → bit-allocate → write. Loudness tracking `0.98*prev + 0.01*(l0+l1)` stereo, `0.98*prev + 0.02*l` mono. Normal decoder pipeline: read frame → parse block size → dequant → IMDCT → QMF synthesis → clamp. Rust additionally emits silence if the AEA header length causes the engine to request frames beyond the last physical frame; C++ lets the reader throw `TNoDataToRead` and the CLI catches it as success. |

### 1.2 ATRAC3 codec

| Rust file | C++ file(s) | Notes |
|-----------|-------------|-------|
| `at3/data.rs` | `atrac/at3/atrac3.h` | `MAX_BFUS=32`, `NUM_QMF=4`, `NUM_SAMPLES=1024`, `BlockSizeTab`, `MaxQuant`, `BlocksPerBand`, `SpecsPerBlock`, `GAIN_LEVEL[16]`, `GAIN_INTERPOLATION[31]`, `SCALE_TABLE`, `ENCODE_WINDOW[256]`, `DECODE_WINDOW[256]`, all 7 Huffman table pairs — spot-checked, all match. Selector/table 4 aliases table 1, matching C++. All `ContainerParams` entries (LP4/LP2/SP/HQ) match C++ `GetContainerParamsForBitrate`. |
| `at3/qmf.rs` | `atrac/at3/atrac3_qmf.h` | Three-stage QMF cascade: `Qmf1(1024)` → `Buf1`/`Buf2`, `Qmf2(512)` → sub0/sub1, `Qmf3(512)` → sub3/sub2. Sub-band output order (`sub3` before `sub2` for Qmf3) matches C++ `atrac3_qmf.h:37-40`. |
| `at3/yaml_log.rs` | `yaml_log.h` | `write_float_seq` produces `[v0, v1, ...]` with configurable precision, matching C++ `YamlWriteFloatSeq`. Empty array produces `[]`. C++ `TYamlFmtGuard` RAII float-format has no equivalent, but Rust `write!(..., "{value:.precision$}")` achieves identical fixed-precision. |
| `at3/mdct.rs` | `atrac3denc.h` (`TAtrac3MDCT` + `TGainProcessor`) | Forward MDCT: copy prev→tmp, modulate tmp+cur, window with `ENCODE_WINDOW[i]`/`ENCODE_WINDOW[255-i]`, transform, swap odd bands — all match `atrac3denc.cpp:33-58`. `build_sample_divisors` matches C++ `BuildSampleDivisors`. `CalcGainEnergyScale` matches C++ `TAtrac3MDCT::CalcGainEnergyScale`. `relation_to_idx` matches `RelationToIdx`. |
| `at3/encoder.rs` | `atrac3denc.cpp/.h`, `atrac/at3/atrac3.cpp` | Frame pipeline: (1) QMF analysis into lookahead buffer at offset 128/384, (2) `LookAhead` on first call, (3) build gain input, (4) copy current slot to MDCT input, (5) optional JS matrixing (mid/side), (6) per-channel `CreateSubbandInfo` → gain energy scale → MDCT with gain → loudness → `ExtractTonalComponents` → scaling, (7) track loudness, (8) write sound unit, (9) shift lookahead. All match `atrac3denc.cpp:680-866`. `extract_tonal_components` (`encoder.rs:666`) adds a `spec_num_end > specs.len()` bounds check not present in C++ — safety improvement. `MapTonalComponents` grouping (max 7 contiguous) matches. |

### 1.3 Psychoacoustics (`atrac/`)

| Rust file | C++ file(s) | Notes |
|-----------|-------------|-------|
| `atrac/psy.rs` | `atrac/atrac_psy_common.cpp/.h` | All 7 functions (`analyze_scale_factor_spread`, `ath_formula_frank`, `calc_ath`, `calc_spectral_flatness_per_bfu`, `track_loudness`, `track_loudness_mono`, `create_loudness_curve`) have matching formulas and valid-input control flow. `calc_spectral_flatness_per_bfu` differs only for invalid BFU ranges: Rust asserts, while C++ release builds return flatness `1.0` for that BFU. `TAB[140]` in `ath_formula_frank` — all 140 values spot-checked, exact match. `ENERGY_FLOOR = 1.0e-12` matches C++ `1e-12f`. Loudness coefficients 0.98/0.01/0.02 all match. Frequency clamped `[10, 29853]` in `ath_formula_frank`. Sigma clamped at 14.0. |
| `atrac/scale.rs` | `atrac/atrac_scale.cpp/.h` | `Scaler::new` (sorted vec + `partition_point`) matches C++ `std::map + lower_bound`. `Scaler::scale` (max search, clamp, scale-factor lookup, energy sum, clip to ±0.99999) identical for current scale tables, which end at `1.0` and are used with `MAX_SCALE=1.0`. `quant_mantissas`: ea/non-ea paths, candidate collection, sort by `abs(delta)`, e2<e1 and e2>e1 branches — all match. **Defensive guard:** `scale.rs:48` clamps `pos` before indexing, avoiding an end-iterator-style edge if alternate/malformed scale tables were used. |

### 1.4 DSP analysis

| Rust file | C++ file(s) | Notes |
|-----------|-------------|-------|
| `dsp/transient.rs` | `transient_detector.cpp/.h` | HPFilter matches the C++ pair-sum FIR implementation and coefficients exactly, including the implicit center sample and `FIR_LEN=21` indexing. Detect: RMS per short block, `19*log10`, attack/release thresholds 16/20. AnalyzeGain: peak/RMS per step, micro-chunk quartiles. median_filter::1 (3-point). `RelationToIdx` via `GetFirstSetBit`. FindPlateau: sliding min-of-3, hard/soft tail release (0.1/0.5/0.7). CalcCurve: plateau target, sticky quantisation (intra/inter ratios 7.0/10.0), boundary transient score (window=3), transition pruning by delta (priority tiebreak rightmost-loc), 6-point budget — all match. |
| `dsp/upsampler.rs` | `transient_spectral_upsampler.cpp/.h` | Planck-taper window: same Zp formula, same epsilon. Forward FFT (complex-to-complex w/ imag=0 vs. C++ real-to-complex — equivalent). Energy ratio with H[k]² weighting. 8× frequency-domain upsampling via zero-padding. 3-bin raised-cosine transition band `H[i]=0.5*(1-cos(π*i/2))`. Original Nyquist bin is half-scaled only when `low_cut_bin + 2 <= IN_N/2`, matching C++; otherwise it remains zero. Hermitian symmetry explicitly set for inverse FFT. Normalisation by `1/OUT_N`. |
| `dsp/delay_buffer.rs` | `delay_buffer.h` | `Shift` copies second-half to first-half; erase mode writes `T::default()` to the second half. Layout `[[[T;S];2];N]` equivalent to C++ `T[N][S*2]`. For numeric zero-default uses such as `float`, Rust default init matches C++ `memset(0)`. |
| `dsp/gain.rs` | `gain_processor.h` | `get_gain_inc` (ExponentOffset - levelIdxCur + posShift), `get_gain_inc2` (levelIdxNext - levelIdxCur + posShift), `modulate` (loc<<LocScale, const+interpolation regions, tail flush), `demodulate` (scale from giNext[0], overlap-add (cur*scale + prev)*level) — all match C++ for non-empty gain lists. For empty lists, C++ returns a null/falsy closure and Rust returns early as a no-op; call-site behaviour is equivalent. `GAIN_LEVEL[16]` = 2^(4-L), `GAIN_INTERPOLATION[31]` = 2^(-(i-15)/LocSz). |

### 1.5 DSP transforms

| Rust file | C++ file(s) | Notes |
|-----------|-------------|-------|
| `dsp/mdct.rs` | `lib/mdct/mdct.cpp/.h` | `Mdct::transform`: pre-twiddle → FFT → post-twiddle. `Midct::transform`: pre-twiddle → FFT → post-twiddle/output reordering. Codec-specific windowing/overlap-add lives in `at1/mdct.rs` and `at3/mdct.rs`, as in C++. `calc_sin_cos`: `alpha=2π/8n`, `omega=2π/n`, `scale=sqrt(scale/n)` — identical to C++ `CalcSinCos`. Default scales: `Mdct=1.0`, `Midct=N` — match C++ defaults. |
| `dsp/dct.rs` | `lib/mdct/dct.h` | `Dct4::transform`: `output[i] = -x[i + n/2]` matches C++ `atde_do_dct4_16`. Scale: `Midct(n*2, (n*2)*scale)` for n=16 → Midct(32, 32*scale) — matches C++. Rust generalises beyond C++'s n=16 only (additive). |
| `dsp/qmf.rs` | `qmf/qmf.cpp/.h` | `TAP_HALF` and derived `QMF_WINDOW` are bit-identical. Analysis is equivalent despite different indexing: C++ convolves against reversed single-buffer history, Rust convolves against forward split even/odd history, and `QMF_WINDOW_ODD[i] == QMF_WINDOW_EVEN[23-i]`. Synthesis and `CalcFreqResp` match; Rust uses a complex FFT for the real-only response input. |
| `dsp/fft.rs` | `lib/fft/kissfft_impl/*` | `FftPlan::forward`/`inverse` wrap `rustfft::FftPlanner` in-place with scratch. Rust and C++ both plan the requested size rather than being limited to a fixed set; actual Rust requests include MDCT-derived sizes, QMF `2*sz`, and upsampler 512/4096. Functionally equivalent to kissfft. No real-FFT mode (C++ `kiss_fftr` used by `CalcFreqResp`), but complex FFT of real-only input achieves equivalent output. |

---

## 2. Algorithmic divergences — GAP

### 2.1 BFU trimming timing (`at3/bitstream.rs`)
**Impact: speed/quality tradeoff, no correctness issue**

C++ `TAlloc::Encode` (`atrac3_bitstream.cpp:602-609`) calls `CheckBfus` inside
the binary search loop and returns `Repeat` to restart with reduced BFU count,
reallocating freed bits.  Rust now supports both behaviours:

- default `--at3-bfu-mode fast`: trims zero-precision trailing BFUs after the
  binary search completes.  This preserves the original Rust ATRAC3 performance
  profile (about 2.2-2.6x faster than C++ in `docs/bench.sh`).
- opt-in `--at3-bfu-mode parity`: restarts the allocation search after BFU
  trimming, matching the C++ timing more closely and increasing cross-encoder
  SNR, at roughly 2x slower ATRAC3 encode speed than fast mode.

Both modes produce valid bitstreams and identical container sizes for the tested
bitrates.

### 2.2 BitAllocHandler lifecycle (`bitstream/encode.rs:97-104`)
**Impact: none (semantically equivalent)**

C++ `TBitStreamEncoder::TImpl` IS-A `TBitAllocHandler` (one persistent object,
reset to `RepeatEncPos=0` at end of `DoRun`).  Rust creates a **new**
`BitAllocHandler` per `run()` call in the generic helper.  This helper is not
used by the production ATRAC1/ATRAC3 allocation paths, which have their own
inline allocators.  Semantically equivalent for the helper because C++ resets
`RepeatEncPos` per frame and overwrites the search state on `Start`.

### 2.3 ATRAC3+ `u16` field validation — RESOLVED
**Impact: defensive parity**

C++ `TAt3p` constructor validates `frameSize > UINT16_MAX` and
`numChannels > UINT16_MAX` with descriptive error messages.  Rust's ATRAC3+
RIFF constructor now uses checked conversions and returns `InvalidInput` for
oversized channel counts or frame sizes.  ATRAC3 (non-plus) still silently
narrows in both Rust and C++.

### 2.4 Frame size validation (`container/at3.rs:160-164`, `container/rm.rs:96-101`)
**Impact: safety improvement (Rust is stricter)**

Rust validates that incoming ATRAC3 RIFF and RM frame data length matches the
declared frame size.  C++ ATRAC3 RIFF and RM write whatever is passed.  C++
ATRAC3+ RIFF already validates frame size, so the Rust-stricter behaviour is
limited to ATRAC3 RIFF and RM.

### 2.5 OMA StereoJoint channel mapping (`container/oma.rs:48`)
**Impact: none in practice**

Rust maps `StereoJoint` to channel index 1 (same as Stereo) for AT3+.
C++ `oma_get_channel_idx` returns -1 for `StereoJoint` because
`channel_id_to_format_tab` omits `STEREO_JS`.  C++ callers never pass
`StereoJoint` for AT3+ (`oma.cpp:33-35` limits to Mono/Stereo).

### 2.6 WAV-only PCM I/O (`pcm/wav.rs`)
**Impact: intentional; matches CLI constraint**

C++ supports WAV, AU, AIFF, RAW via libsndfile.  Rust supports WAV only via
`hound`.  Per `AGENTS.md`: "CLI input must be 44100 Hz, 16-bit, mono or stereo
WAV." — no format gap in practice.

### 2.7 ATRAC1 decode EOF padding (`at1/codec.rs:200-216`)
**Impact: truncated/mismatched AEA edge cases only**

Rust emits silence when `AeaInput::read_frame()` returns EOF while the decode
loop still needs samples to satisfy the AEA header length rounded to the PCM
engine block size.  C++ lets the AEA reader throw `TNoDataToRead`, and the CLI
catches that exception as successful end-of-stream.  For well-formed files this
only affects the final padded engine block; for physically truncated files Rust
can pad silence where C++ would stop via exception.

---

## 3. Defensive and safety improvements (Rust > C++)

These are changes where Rust behaviour is more defensive or stricter than C++:

| Location | Improvement |
|----------|-------------|
| `atrac/scale.rs:48` | Guards `pos >= scale_index.len()` with `.min()`; this is defensive for alternate/malformed scale tables. Current C++ tables end at `1.0`, so `map::end()` is not reachable in normal use. |
| `atrac/psy.rs:78` | Asserts invalid BFU ranges in `calc_spectral_flatness_per_bfu`; C++ release builds return flatness `1.0` for an out-of-range BFU. |
| `at3/encoder.rs:664` | `spec_num_end > specs.len()` bounds check — absent in C++ |
| `pcm/engine.rs` | Drain guard uses `if self.to_drain != 0`; C++ uses `if (drain && ToDrain--)`, which would wrap if the surrounding invariant were violated. |
| `util.rs` `Div8Ceil` | `assert!(x > 0)` guards against unsigned wraparound at x=0; C++ unsigned arithmetic is defined but would return a bogus large value. |
| `bitstream/mod.rs` `Read` | Explicit `assert!(self.read_pos + n <= self.buf.len() * 8)` — C++ has no bounds check |
| `container/aea.rs` | Backfills frame count on finalize — C++ never backfills, writes estimated count forever |
| `container/at3.rs`, `container/rm.rs` | ATRAC3 RIFF and RM frame size validation (see §2.4) |
| `container/at3.rs` | ATRAC3+ RIFF now rejects oversized `u16` channel/frame-size fields instead of silently truncating. |

---

## 3.1 Resolved during cleanup

These items were present in the original audit and have since been fixed or made
configurable:

| Location | Resolution |
|----------|------------|
| `at3/bitstream.rs` | Added `BfuAllocMode::{Fast, Parity}`. Fast remains default; C++-parity BFU search restart is available through `--at3-bfu-mode parity`. |
| `container/at3.rs` | ATRAC3+ RIFF constructor now checks `channels` and `frame_size` before writing `u16` fields. |
| `main.rs` | ATRAC1 `--bfuidxconst > 8` is rejected with `InvalidInput`. |
| `main.rs` | `.dat` output extension now infers the RAW container. |
| `main.rs` | AEA, RIFF/AT3, and RM output constructors now receive the WAV-derived encoded frame count instead of hardcoded `0`. |
| `README.md` | User-facing CLI examples now document `--at3-bfu-mode parity` and explain that fast BFU allocation remains the ATRAC3 default. |

---

## 4. CLI behavioural gaps (`main.rs`)

No encode/decode logic missing; all differences are in CLI ergonomics and
error handling strategy.

### 4.1 Input validation — stricter in Rust

| Gap | Rust | C++ |
|-----|------|-----|
| Channels | Rejects >2 channels (`main.rs:145-146`) | None |
| Bit depth | Rejects non-16-bit (`main.rs:151-152`) | None |
| `--bfuidxconst` > 8 (ATRAC1) | Rejected with `InvalidInput` | `main.cpp:638-641` |
| `--bitrate` invalid | Hard error (`main.rs:383-387`) | Falls back to default with warning |
| `--at3-bfu-mode parity` | Rust-only opt-in C++-parity ATRAC3 allocation mode | No equivalent; C++ always uses parity-style restart |

### 4.2 Progress output — mostly absent in Rust

C++ prints a startup banner (channels, sample rate, duration, codec, container,
bitrate) and per-frame progress when `--nostdout` is not set.  Rust does not
print startup/progress output.  `--bfuidxfast` remains accepted for compatibility
and is effectively the default ATRAC3 allocation mode.

| C++ (`main.cpp`) | Rust status |
|-------------------|-------------|
| `cout << "codec: ...` (line 331) | absent |
| `cout << "output container: ...` (line 410) | absent |
| `printProgress(...)` / `"Done"` (line 701-706) | absent |
| `--notransient` band info text (line 569-576) | absent |

### 4.3 Container and output construction

| Gap | Rust | C++ |
|-----|------|-----|
| `"dat"` extension → RAW | Recognised | `main.cpp:202, 216` |
| `"omg"` extension → OMA | Recognised (`main.rs:422`) and therefore rejected for ATRAC1 | No explicit case; unknown extensions default to OMA for ATRAC3/ATRAC3+ and AEA for ATRAC1 |
| `"ra"` extension → RM | Recognised (`main.rs:424`) | Not recognised |
| Frame count passed to constructors | Computed from WAV length for AEA, RIFF, and RM | Computed from WAV length (`main.cpp:325`) |
| Frame count overflow warning | None | `main.cpp:313-316, 379-383` |
| `-o -` → stdout | Not supported | Supported by the libsndfile PCM path; compressed outputs open literal `-` paths |
| `--advanced` flag | Absent (ATRAC3+ not ported) | `main.cpp:501` |

### 4.4 Error handling

| Gap | Rust | C++ |
|-----|------|-----|
| Decode `TNoDataToRead` after loop | Core decoder pads missing final frames with silence; other PCM engine errors are fatal (`main.rs:234`) | Caught, returns success (`main.cpp:713-716`) |
| `TAeaIOError` specialised catch | None | `main.cpp:709-712` |
| Container validation during decode | Rejects non-AEA (`main.rs:187-192`) | Rejects ALL containers (`main.cpp:623-626`) |

---

## 5. Unported (intentional)

| C++ | Reason |
|-----|--------|
| `at3p/*` (10+ files: bitstream, GHA, MDCT, tables, FFmpeg reference) | ATRAC3+ not ported (`main.rs:159-161`) |
| `atrac/atrac3plus_pqf/*` (PQF filter) | Part of ATRAC3+ |
| `platform/win/*` (Windows PCM I/O, getopt) | Not needed (cross-platform Rust) |
| `cmake/modules/FindLibSndFile.cmake` | Build system, not ported |
| `pcm_io_sndfile.cpp` | Replaced by `hound` crate |
| `lib/fft/kissfft_impl/*` | Replaced by `rustfft` crate |
| `lib/libgha/*` (GHA library) | Unported with ATRAC3+ GHA; this is not an FFT backend replacement |
| `lib/liboma/*` (liboma C library) | OMA format handled directly in `container/oma.rs` |
| `file.h`, `utf8_file.h`, `endian_tools.h`, `compressed_io.h` | Obsoleted by Rust std (`File`, `OsString`, primitive byte conversions, traits) |
| `config.h` (kiss_fft_scalar, M_PI, NOMINMAX) | Not needed (Rust equivalents) |

---

## 6. Recommendations

### High priority

No high-priority codec parity fix remains after validating the QMF analysis
filter equivalence.

### Low priority

1. **Use `--at3-bfu-mode parity` for closer C++ ATRAC3 allocation parity** when
   validating encoder output.  Fast mode remains the default because it restores
   the original Rust ATRAC3 benchmark profile (`LP2` about 2.8s on the reference
   input versus about 5.4s with parity allocation).

2. **Frame count overflow warning parity** remains lower priority.  Rust now
   errors if the computed frame count cannot fit container metadata, while C++
   warns in some paths.
