# Idiomatic Rust Remediation Plan

Generated from a full-module audit of `atracdenc-core` and `atracdenc-cli`.
Issues are grouped by category, ordered by severity.

---

## Legend

| Symbol | Meaning |
|--------|---------|
| [H] | High severity — correctness, panic risk, or memory waste in hot paths |
| [M] | Medium severity — maintainability, robustness, API smell |
| [L] | Low severity — style, minor duplication, naming |

---

## Phase 1: Eliminate Panics in Production Code

### 1.1 Replace `expect()` with error propagation

Files using `expect()` that crash on recoverable conditions:

**[H] `container/aea.rs:63,82-85`** — `Option<W>.take().expect(...)` anti-pattern
**[H] `container/at3.rs:163,175-178`** — same anti-pattern
**[H] `container/rm.rs:72,76-78`** — same anti-pattern
**[H] `crates/atracdenc-core/src/at1/codec.rs:150-151,203-204`** — panics on I/O write/read
**[H] `crates/atracdenc-core/src/at3/encoder.rs:495`** — panics on frame write
**[H] `crates/atracdenc-core/src/at3/bitstream.rs:528`** — panics on missing SCE context

**Approach (containers):**

The root cause: `into_inner(self)` needs to move `W` out, but `finalize()` takes `&mut self`. This forces `Option<W>`.

Fix: restructure so the writer is moved into `into_inner` unconditionally:

```rust
// Remove Option<W>, store W directly (like oma.rs already does).
// For AEA/AT3/RM (which need back-patching in finalize):
//   - finalize() takes &mut self and writes via &mut W
//   - into_inner(self) takes ownership and returns W
//   - Drop calls finalize() if not already finalized (track with bool)
// This is the pattern oma.rs and raw.rs already use correctly.
```

Affected files:
- `crates/atracdenc-core/src/container/aea.rs` — `AeaOutput<W>`
- `crates/atracdenc-core/src/container/at3.rs` — `At3Output<W>`
- `crates/atracdenc-core/src/container/rm.rs` — `RmOutput<W>`

**Approach (codecs):**

These are constrained by the `process_frame` trait returning `ProcessResult`, not `Result`. Options:

A. Change the trait signature to `-> io::Result<ProcessResult>` — most correct, widest blast radius
B. Log the error and return `ProcessResult::Data` with zeros — less correct, prevents crashes
C. Store the error in `self` and surface it in a subsequent `flush()` call — defer failure

Recommendation: **Option A** — change `process_frame` in `container/mod.rs` to return `io::Result<ProcessResult>`. Propagate through `at1/codec.rs`, `at3/encoder.rs`, `pcm/engine.rs`, and `cli/main.rs`.

### 1.2 Replace bare `unwrap()` in transient.rs

**[H] `crates/atracdenc-core/src/dsp/transient.rs:301,326`** — `*input.last().unwrap()`
**[H] `crates/atracdenc-core/src/dsp/transient.rs:367-368`** — `subframe_low.unwrap()`, `subframe_high.unwrap()`

Fix:
```rust
// Line 301, 326: replace with
*input.last().unwrap_or(&0.0)

// Lines 367-368: replace with destructuring
if let (Some(low), Some(high)) = (subframe_low, subframe_high) {
    // ... use low and high directly
}
```

### 1.3 Guard `TAB` indexing in psy.rs

**[H] `crates/atracdenc-core/src/atrac/psy.rs:48-50`** — `TAB[index]` / `TAB[index + 1]` can OOB

Fix:
```rust
let index = (freq_log.floor() as usize).min(TAB.len() - 2);
```

### 1.4 Fix `channels()` in rm.rs

**[H] `crates/atracdenc-core/src/container/rm.rs:112-114`** — returns hardcoded `0`

Fix: store `num_channels: u16` in `RmOutput` (set in `new()`), return it from `channels()`.

---

## Phase 2: Fix Error Type Deficiencies

### 2.1 PcmEngineError missing trait impls

**[H] `crates/atracdenc-core/src/pcm/engine.rs:7-11`**

Add:
```rust
impl std::fmt::Display for PcmEngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongReadBuffer => write!(f, "channel count mismatch between buffer and reader"),
            Self::NoDataToRead => write!(f, "no more data to read from input"),
            Self::DataNotReady => write!(f, "output data not yet ready"),
        }
    }
}
impl std::error::Error for PcmEngineError {}
```

### 2.2 PcmEngineError variants are overloaded

**[H] `crates/atracdenc-core/src/pcm/engine.rs:10`** + call sites in `wav.rs:44,55,96,116`

`WrongReadBuffer` is used for: channel mismatch, hound read errors, and hound write errors.

Fix: Add proper variants:
```rust
pub enum PcmEngineError {
    ChannelMismatch,
    NoDataToRead,
    DataNotReady,
    IoError(std::io::Error),   // wraps hound::Error via .map_err(PcmEngineError::IoError)
}
```

### 2.3 Hound errors silently discarded

**[H] `crates/atracdenc-core/src/pcm/wav.rs:55,116`**

Fix: once `PcmEngineError` has an `IoError` variant, use it:
```rust
Some(Err(e)) => return Err(PcmEngineError::IoError(e.into())),
```

### 2.4 CLI swallows original errors

**[M] `crates/atracdenc-cli/src/main.rs:145,210-215`**

Fix: include the underlying error in the message:
```rust
.map_err(|e| invalid_input(format!("unable to open input file {}: {e}", input.display())))
```

---

## Phase 3: Eliminate Hot-Path Heap Allocations

### 3.1 MDCT scratch buffers

**[H] `crates/atracdenc-core/src/at1/mdct.rs:61,118-119`** — `vec![0.0; 512]` every call
**[M] `crates/atracdenc-core/src/at3/mdct.rs:82`** — `vec![0.0; MDCT_SZ]` every call

Fix: add pre-allocated scratch buffers to the struct:
```rust
pub struct Atrac1Mdct {
    mdct512: Mdct,
    midct512: Midct,
    scratch: Vec<f32>,    // 512 elements, allocated once in new()
    inv_buf: Vec<f32>,    // 512 elements, allocated once in new()
}
```

### 3.2 Redundant `.to_vec()` on transform results

**[H] `crates/atracdenc-core/src/at1/mdct.rs:73-81,127-133`**
**[M] `crates/atracdenc-core/src/at3/mdct.rs:125`**

`Mdct::transform()` returns `&[f32]` (borrows internal buffer). Calling `.to_vec()` copies it.

Fix: use `copy_from_slice` into the pre-allocated scratch buffer, or use the borrow directly where possible.

### 3.3 Per-frame allocations in codec frame processing

**[M] `crates/atracdenc-core/src/at1/codec.rs:80-83,86-89`** — multiple Vecs per process_frame
**[M] `crates/atracdenc-core/src/at3/encoder.rs:454`** — energy Vec for MDCT specs
**[M] `crates/atracdenc-core/src/at3/bitstream.rs:196`** — clone of out_buffer on every frame

Fix: pre-allocate buffers on the encoder struct. Use `.clear()` + `.extend()` instead of allocating fresh Vecs.

### 3.4 Container write_frame clones

**[L] `crates/atracdenc-core/src/container/aea.rs:80-81`** — `data.to_vec()` every frame
**[L] `crates/atracdenc-core/src/container/raw.rs:28-29`** — same pattern

Fix: conditionally clone only when padding is needed:
```rust
let frame = if data.len() < AEA_FRAME_SIZE {
    let mut f = data.to_vec();
    f.resize(AEA_FRAME_SIZE, 0);
    f
} else {
    data.to_vec()
};
// or better: take a Vec by value to avoid cloning in the common (exact-size) case
```

---

## Phase 4: Error Handling Consistency

### 4.1 `assert!` instead of `Result` in library code

**[H] `crates/atracdenc-core/src/pcm/engine.rs:144`** — unconditional `assert!(!drain)`

Fix: change to `debug_assert!(!drain)` (elided in release) or return a `Result`.

**[M] `crates/atracdenc-core/src/at3/bitstream.rs:116-482`** — ~20 assertions panic on malformed input

These functions already return `io::Result`. Fix: convert each assertion to return `Err(...)`:
```rust
// Instead of:
assert!(sce.subband_info.len() == NUM_QMF, "wrong subband info");
// Use:
if sce.subband_info.len() != NUM_QMF {
    return Err(io::Error::new(io::ErrorKind::InvalidData, "wrong subband info"));
}
```

**[L] Many DSP `assert!` / `assert_eq!` calls** across `at1/mdct.rs`, `at3/mdct.rs`, `dsp/dct.rs`, `dsp/qmf.rs`, `dsp/transient.rs`, `dsp/gain.rs`, `dsp/upsampler.rs`

These are for internal invariant checking (e.g., buffer length validation). For most, `debug_assert!` is sufficient — elide in release, catch bugs in tests. For public API functions, prefer `Result`.

### 4.2 Utility function assertions

**[M] `crates/atracdenc-core/src/util.rs:25,30`** — `assert!` in `div8_ceil` and `calc_median`

Fix: return `Option`:
```rust
pub fn div8_ceil(x: u32) -> Option<u32> { x.checked_sub(1).map(|v| v / 8 + 1) }
pub fn calc_median(input: &[f32]) -> Option<f32> { ... }

// Update callers to handle None or expect (caller decides).
```

### 4.3 Sentinel values instead of `Option`

**[L] `crates/atracdenc-core/src/util.rs:18-20`** — `get_first_set_bit(0)` returns sentinel `0`

Fix: return `Option<u16>` with `None` for input `0`.

**[M] `crates/atracdenc-core/src/at1/bitalloc.rs:203`** — `999.0` sentinel instead of `f32::MAX`

Fix: use `f32::MAX` or restructure to use `Option`.

**[L] `crates/atracdenc-core/src/at3/encoder.rs:669`** — `best_score = -1.0` sentinel

Fix: use `Option<f32>`.

---

## Phase 5: Remove Dead / Unused Code

### 5.1 Dead bitstream encoding module

**[M] `crates/atracdenc-core/src/bitstream/encode.rs`** — `BitStreamEncoder`, `BitAllocHandler`, `BitStreamPartEncoder` are never used outside their own tests.

Options:
- Remove the file entirely if superseded by `at3/bitstream.rs`
- Add `#[allow(dead_code)]` with a comment if it's scaffolding for a future feature
- Mark as `#[cfg(test)]` if only tests remain useful

### 5.2 Dead methods in encode.rs

**[M]** `BitAllocHandler::check()` (line 54), `BitAllocHandler::cur_global_consumption()` (line 75)

Remove or document their purpose.

### 5.3 Dead CLI flags

**[M] `crates/atracdenc-cli/src/main.rs:70,76,84`** — `--bfuidxfast`, `--nostdout`, `--mono`

These are parsed but never read in any execution path. Either:
- Wire them up to their intended behavior
- Remove them to avoid user confusion
- Mark as hidden with `#[arg(hide = true)]` and a comment if planned for future

### 5.4 Unused `#[derive(Clone)]` on CliOptions

**[L] `crates/atracdenc-cli/src/main.rs:53`** — `CliOptions` is never cloned. Remove the derive.

---

## Phase 6: Reduce Code Duplication

### 6.1 Mid/Side matrixing triplicated

**[H] `crates/atracdenc-core/src/at3/encoder.rs:128-139,150-161,738-747`**

Three implementations of `(l + r) * 0.5, (l - r) * 0.5`. The public free function `matrixing` (line 738) is already tested.

Fix: extract `matrixing` as a shared helper and call it from all three sites.

### 6.2 Subframe divisor builders duplicated

**[L] `crates/atracdenc-core/src/at3/encoder.rs:503-531`** (`build_subframe_divisors`)
**[L] `crates/atracdenc-core/src/at3/mdct.rs:237-261`** (`build_sample_divisors`)

Identical logic modulo array sizes (32 vs 256). Unify into a shared utility taking a generic size.

### 6.3 Planck window duplicated in upsampler

**[L] `crates/atracdenc-core/src/dsp/upsampler.rs`** — window logic in `new()` duplicated in test `planck_windowed_rms`.

Extract a shared window-generating function.

### 6.4 `block_band` logic duplicated in encoder.rs

**[L] `crates/atracdenc-core/src/at3/encoder.rs:503-531`** and `crates/atracdenc-core/src/at3/bitstream.rs:886-898`

Both contain identical block-band detection. Consolidate.

---

## Phase 7: Naming and Style Improvements

### 7.1 Abbreviated names

| File | Current | Suggested |
|------|---------|-----------|
| `util.rs:5` | `invert_spectr` | `invert_spectrum` |
| `util.rs:11` | `inverted_spectr` | `inverted_spectrum` |
| `util.rs:17` | `get_first_set_bit` | `msb_index` / `fls` |
| `util.rs:24` | `div8_ceil` | `ceil_div_8` / `div_8_ceil` |
| `util.rs:29` | `calc_median` | `median` or `compute_median` |
| `util.rs:36` | `calc_energy` | `energy` |
| `at1/codec.rs:23` | `LOUD_FACTOR` | `LOUDNESS_FACTOR` |
| `at3/encoder.rs:30` | `LOUD_FACTOR` | `LOUDNESS_FACTOR` |
| `container/rm.rs:9` | `RMF_HEADER_SZ` | `RMF_HEADER_SIZE` |

Note: domain acronyms like `qmf`, `mdct`, `bfu`, `imdct` are acceptable as they match the ATRAC specification.

### 7.2 Misleading names

| File | Line | Current | Issue |
|------|------|---------|-------|
| `atrac/scale.rs:73` | `max_bfus` | Variable name implies BFU count but holds block count |
| `at3/encoder.rs:669` | `best_score` | Sentinal `-1.0` disguised as a score value |

### 7.3 Remove redundant `swap_array` wrapper

**[M] `crates/atracdenc-core/src/util.rs:1-3`**

`swap_array` is a one-line wrapper around `.reverse()`. Replace all 4 call sites (`at1/mdct.rs:87,124`, `at3/mdct.rs:96,122`) with `.reverse()` and remove the function.

### 7.4 Manual `Default` impl in encode.rs

**[L] `crates/atracdenc-core/src/bitstream/encode.rs:16-30`**

Replace with `#[derive(Default)]`.

---

## Phase 8: C-Style Loop → Iterator Conversions

### 8.1 MDCT transform loops

**[L] `crates/atracdenc-core/src/dsp/mdct.rs:49-70,138-169`**

`while n < n4 { ... n += 2; }` → `for n in (0..n4).step_by(2)`

### 8.2 Bit allocation state machine

**[H] `crates/atracdenc-core/src/at1/bitalloc.rs:258-279`** — `get_max_used_bfu_id`

Complex C-ported state machine. Refactor into smaller functions with descriptive names.

### 8.3 Transient detection loops

**[M] `crates/atracdenc-core/src/dsp/transient.rs:40-69`** — `hp_filter` manual FIR loops
**[M] `crates/atracdenc-core/src/dsp/transient.rs:391-396`** — reverse loop finding target_sf

### 8.4 ATH computation

**[M] `crates/atracdenc-core/src/at1/bitalloc.rs:195-209`** — `calc_at1_ath` triple-nested manual indexing

Can use `chunks`/`flat_map` iterator chains.

### 8.5 Gain processor loops

**[M] `crates/atracdenc-core/src/dsp/gain.rs:49-68,87-108`** — `while pos < last_pos` C-style loops

### 8.6 Tonal component grouping

**[M] `crates/atracdenc-core/src/at3/bitstream.rs:423-468`** — `group_tonal_components` manual state loop
**[M] `crates/atracdenc-core/src/at3/encoder.rs:711-735`** — `map_tonal_components` C-style grouping

---

## Phase 9: Type Safety Improvements

### 9.1 `u16`/`u32`/`usize` inconsistencies

**[M] `crates/atracdenc-core/src/atrac/scale.rs:95-96`** — `first: u32, last: u32` cast to `usize`

Fix: accept `usize` directly since all callers have `usize` values.

**[L] `crates/atracdenc-core/src/pcm/engine.rs:21`** — `buf_size: u16` cast to `usize`

Accept `usize` and validate bounds at the call site.

**[L] `crates/atracdenc-core/src/pcm/engine.rs:42-44`** — `channels` stored as `usize`, returned as `u16`

Store as `u16` with validation, or return `usize`.

### 9.2 `i32::abs()` overflow risk

**[L] `crates/atracdenc-core/src/atrac/scale.rs:143,171`** — `mantissas[f].abs()` on `i32`

Use `mantissas[f].unsigned_abs() as f32` (or `.checked_abs()`) to avoid `i32::MIN` panic.

### 9.3 Float-to-int cast in bitalloc

**[M] `crates/atracdenc-core/src/at1/bitalloc.rs:235-244`** — `as i32` cast on potentially out-of-range float

Clamp before casting:
```rust
let clamped = tmp.round_ties_even().clamp(i32::MIN as f32, i32::MAX as f32) as i32;
```

### 9.4 `bitwise &` on bool

**[M] `crates/atracdenc-core/src/at3/bitstream.rs:257`** — `&` used on `bool` operands

Use `&&` (short-circuiting logical AND).

---

## Phase 10: Miscellaneous Fixes

### 10.1 Repeated float constant computation

**[M] `crates/atracdenc-core/src/pcm/wav.rs:110-111`** — `32767.0 / 32768.0` and `32768.0` computed per sample

Hoist to `let` bindings outside the loop.

### 10.2 `step_by(0)` panic vector

**[L] `crates/atracdenc-core/src/pcm/engine.rs:113-115,135`** — no guard for `step == 0`

Add `if step == 0 { return Err(...) }`.

### 10.3 Redundant `clone()` on `Range<usize>`

**[L] `crates/atracdenc-core/src/dsp/qmf.rs:219`** — `Range<usize>` is `Copy`

Remove `.clone()`.

### 10.4 Channel inconsistency in AEA

**[M] `crates/atracdenc-core/src/container/aea.rs:140-143`** — `length_in_samples()` and `channels()` interpret channel byte `0` differently

Align both to treat `0` as mono (`1` channel), matching real AEA files.

### 10.5 `ProcessMeta` newtype for single field

**[M] `crates/atracdenc-core/src/pcm/engine.rs:75-78`**

Either pass `channels: u16` directly or expand `ProcessMeta` with additional metadata fields.

### 10.6 Boolean flag anti-pattern in quant_mantissas

**[M] `crates/atracdenc-core/src/atrac/scale.rs:93-192`** — `ea: bool` parameter selects between two code paths

Split into `quant_mantissas` and `quant_mantissas_with_energy_adjustment`.

### 10.7 `tcsgn` / `tcgn_check` cryptic abbreviations

**[L] `crates/atracdenc-core/src/at3/bitstream.rs:309,413`**

Rename to `total_subgroups` and `subgroups_written`.

### 10.8 `cont` declared uninitialized

**[L] `crates/atracdenc-core/src/bitstream/encode.rs:104`** — `let mut cont;` then assigned in-loop

Initialize at declaration: `let mut cont = false;`.

### 10.9 `f32` precision in RMS computation

**[L] `crates/atracdenc-core/src/dsp/transient.rs:8-9`** and `upsampler.rs:145`

Accumulate in `f64` for long slices to avoid catastrophic cancellation.

### 10.10 `Range<usize>.clone()` fix

**[L] `crates/atracdenc-core/src/dsp/qmf.rs:219`** — test code: `range.clone()` where `Range` is `Copy`

Remove `.clone()`.

---

## Implementation Order (Recommended)

| Order | Phase | Rationale |
|-------|-------|-----------|
| 1 | Phase 1 | Eliminate production panics first — highest user impact |
| 2 | Phase 2 | Fix error types — enables clean fixes for many other issues |
| 3 | Phase 3 | Hot-path allocations — measurable performance improvement |
| 4 | Phase 4 | Error handling consistency — makes all panics auditable |
| 5 | Phase 5 | Remove dead code — reduces audit surface |
| 6 | Phase 6 | Deduplication — maintainability |
| 7 | Phase 9 | Type safety — prevents future bugs |
| 8 | Phase 7 | Naming — cosmetic, low risk |
| 9 | Phase 8 | Iterator conversions — readability, low risk if tests pass |
| 10 | Phase 10 | Miscellaneous — clean up remaining nits |

## Verification

After each phase:
```bash
cargo build --release -p atracdenc-cli   # must compile
cargo test                                # all tests must pass
cargo clippy -- -D warnings               # no new warnings
cargo fmt -- --check                      # formatting unchanged
```
