# 05 — Shared ATRAC components: scaler/quantizer, psychoacoustic helpers

## Goal

Port the codec-shared spectral coding pieces: scale-factor selection + block scaling + mantissa quantization, and the psychoacoustic helpers (ATH curve, scale-factor spread, per-BFU spectral flatness, loudness tracking).

## Prerequisites

Phase 02 (util).

## C++ sources

| C++ file | Lines | Rust target |
|---|---|---|
| `atracdenc/src/atrac/atrac_scale.{h,cpp}` | 45+199 | `atracdenc-core/src/atrac/scale.rs` |
| `atracdenc/src/atrac/atrac_scale_ut.cpp` | 67 | `#[cfg(test)]` in `scale.rs` |
| `atracdenc/src/atrac/atrac_psy_common.{h,cpp}` | 58+201 | `atracdenc-core/src/atrac/psy.rs` |
| `atracdenc/src/atrac/atrac_psy_common_ut.cpp` | 376 | `#[cfg(test)]` in `psy.rs` |

## Steps

### 1. `atrac/scale.rs`

C++ API:
- `TScaledBlock { ScaleFactorIndex: u8, Values: Vec<f32>, Energy: f32 }`
- `TScaler<TBaseData>`: builds a `map<float, u8>` from the codec's scale table (`TBaseData::ScaleTable`); `Scale(in, len) -> TScaledBlock` picks the smallest scale factor covering the block max and normalizes values; `ScaleFrame(specs, blockSizeMod) -> Vec<TScaledBlock>` slices the spectrum per BFU using `TBaseData` layout tables.
- `QuantMantisas(in, first, last, mul, ea, mantisas) -> float`: quantizes to integer mantissas; `ea` enables the energy-preserving adjustment loop; returns the (energy) correction.

Rust design — replace the CRTP `TBaseData` with an explicit params struct (the data tables come from each codec in phases 07–09):

```rust
pub struct ScaledBlock { pub scale_factor_index: u8, pub values: Vec<f32>, pub energy: f32 }

pub struct ScaleTableParams<'a> { pub scale_table: &'a [f32] }  // codec scale table

pub struct Scaler { scale_index: BTreeMap<OrderedF32, u8> /* or sorted Vec<(f32,u8)> + binary search */ }
impl Scaler {
    pub fn new(scale_table: &[f32]) -> Self;
    pub fn scale(&self, input: &[f32]) -> ScaledBlock;
    pub fn scale_frame(&self, specs: &[f32], block_layout: &BlockLayout) -> Vec<ScaledBlock>;
}

pub fn quant_mantissas(input: &[f32], first: u32, last: u32, mul: f32, ea: bool,
                       mantissas: &mut [i32]) -> f32;
```

Notes:
- `f32` is not `Ord`; use a sorted `Vec<(f32, u8)>` with `partition_point` instead of `BTreeMap` (the C++ uses `map<float,u8>::lower_bound`-style lookup — read `atrac_scale.cpp` for the exact lookup semantics: which side wins on ties / overflow clamping, and the dB-overflow warning path).
- `BlockLayout` abstracts what C++ pulls from `TBaseData::TBlockSizeMod` + specs tables: per-band log-count tables, BFU start offsets, specs-per-block. Define it here; codecs construct it in phases 07–09. Read `ScaleFrame` in `atrac_scale.cpp` to get the exact fields needed.
- `QuantMantisas` (~80 lines in cpp): port the rounding (`ToInt` → `util::to_int`), clamp, and the `ea` energy-preservation second pass exactly.

### 2. `atrac/psy.rs`

```rust
pub fn analyze_scale_factor_spread(blocks: &[ScaledBlock]) -> f32;      // 0..1 normalized stddev of SFIs
pub fn calc_ath(len: usize, sample_rate: u32) -> Vec<f32>;              // absolute threshold of hearing curve
pub fn calc_spectral_flatness_per_bfu(mdct_energy: &[f32],
                                      specs_start: &[u32],
                                      specs_per_block: &[u32],
                                      num_bfu: usize,
                                      energy_floor: f32) -> Vec<f32>;   // geometric/arithmetic mean ratio per BFU
pub fn track_loudness(prev: f32, l0: f32, l1: f32) -> f32;              // 0.98*prev + 0.01*(l0+l1)
pub fn track_loudness_mono(prev: f32, l: f32) -> f32;                   // 0.98*prev + 0.02*l
pub fn create_loudness_curve(sz: usize) -> Vec<f32>;
```

- Port formulas from `atrac_psy_common.cpp` (ATH polynomial, loudness curve shape) verbatim.
- The C++ template overload `CalcSpectralFlatnessPerBfu<TData>` just forwards codec tables — in Rust the codecs call the explicit-slice version with their tables; no generic needed.
- Default arg `energy_floor = 1e-12` becomes an explicit constant `pub const ENERGY_FLOOR: f32 = 1e-12;` used at call sites.

### 3. Tests to port

- **`atrac_scale_ut.cpp`** (67 L): mantissa quantization preserves energy (with `ea` mode). Port verbatim with the C++ tolerances.
- **`atrac_psy_common_ut.cpp`** (376 L): spectral flatness per BFU for AT1/AT3/AT3P BFU mappings (the test embeds the `SpecsStartLong`/`SpecsPerBlock` tables for each codec — transcribe them into the test; they will later be cross-checked against the codec tables ported in phases 07–09), tone vs noise discrimination. Port all cases.

## Acceptance criteria

- All ported tests green.
- `Scaler::scale` returns the same scale-factor indices as C++ for a table of hand-checked inputs (add 2–3 fixture cases derived by reading the C++ logic, e.g. exact-boundary values).
