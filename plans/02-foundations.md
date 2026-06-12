# 02 — Foundations: bitstream, bit-allocation framework, util

## Goal

Port the dependency-free building blocks every codec uses: the MSB-first bitstream reader/writer, the multi-pass bit-allocation encoder framework, and small numeric helpers.

## Prerequisites

Phase 01 (workspace exists).

## C++ sources

| C++ file | Lines | Rust target |
|---|---|---|
| `atracdenc/src/lib/bitstream/bitstream.{h,cpp}` | 48+88 | `atracdenc-core/src/bitstream/mod.rs` |
| `atracdenc/src/lib/bitstream/bitstream_ut.cpp` | 134 | `#[cfg(test)]` in same module |
| `atracdenc/src/lib/bs_encode/encode.{h,cpp}` | 73+188 | `atracdenc-core/src/bitstream/encode.rs` |
| `atracdenc/src/lib/bs_encode/encode_ut.cpp` | 178 | `#[cfg(test)]` in same module |
| `atracdenc/src/util.h` | 109 | `atracdenc-core/src/util.rs` |
| `atracdenc/src/util_ut.cpp` | 52 | `#[cfg(test)]` in same module |
| `atracdenc/src/lib/endian_tools.h` | 76 | **not ported** — use `to_le_bytes`/`to_be_bytes` at call sites |
| `atracdenc/src/env.{cpp,h}` (FPU rounding) | 55 | **not ported** |
| `atracdenc/src/delay_buffer.h` | 52 | ported in phase 04 with its users |

## Steps

### 1. `bitstream` module — `TBitStream` port

API in C++ (`NBitStream::TBitStream`): growable `Vec` of bytes, MSB-first bit packing, independent write position (`BitsUsed`) and read position (`ReadPos`).

Rust design:

```rust
pub struct BitStream {
    buf: Vec<u8>,
    bits_used: usize,
    read_pos: usize,
}
impl BitStream {
    pub fn new() -> Self;
    pub fn from_bytes(buf: &[u8]) -> Self;       // C++: TBitStream(const char*, int)
    pub fn write(&mut self, val: u32, n: usize); // MSB-first, n <= 32 (C++ asserts n <= 23 per call in places; replicate actual Write semantics from bitstream.cpp)
    pub fn read(&mut self, n: usize) -> u32;
    pub fn size_in_bits(&self) -> usize;         // GetSizeInBits = bits_used
    pub fn buf_size(&self) -> usize;
    pub fn bytes(&self) -> &[u8];
}
pub fn make_sign(val: i32, bits: u32) -> i32;    // sign-extend low `bits` of val
```

Implementation notes:
- Read `bitstream.cpp` for the exact write/read loop (bit-by-bit or chunked across byte boundaries) and replicate behavior, including how partial trailing bytes are zero-padded.
- `make_sign`: `(val << (32 - bits)) >> (32 - bits)` using `i32` arithmetic shifts (`wrapping_shl`/`shr` with care); identical to the C++ union trick.
- No `unsafe` needed anywhere.

### 2. `bitstream::encode` — multi-pass bit-allocation framework

C++: `TBitAllocHandler` (binary search over a `lambda` parameter to hit a target bit budget) + `IBitStreamPartEncoder` pipeline run by `TBitStreamEncoder::Do` with `EStatus::{Ok, Repeat}` control flow. Used by AT1/AT3/AT3+ frame writers.

Rust design:

```rust
pub struct BitAllocHandler { /* target_bits, min/max lambda, cur lambda, consumption bookkeeping */ }
impl BitAllocHandler {
    pub fn start(&mut self, target_bits: usize, min_lambda: f32, max_lambda: f32);
    pub fn cont(&mut self) -> f32;               // C++ Continue()
    pub fn check(&self, got_bits: usize) -> bool;
    pub fn submit(&mut self, got_bits: usize) -> bool; // true = binsearch done
    pub fn cur_global_consumption(&self) -> u32;
}

pub enum EncodeStatus { Ok, Repeat }

pub trait BitStreamPartEncoder<TFrame> {
    fn encode(&mut self, frame: &mut TFrame, ba: &mut BitAllocHandler) -> EncodeStatus;
    fn dump(&mut self, bs: &mut BitStream);
    fn reset(&mut self) {}
    fn consumption(&self) -> u32;
}

pub struct BitStreamEncoder<TFrame> { parts: Vec<Box<dyn BitStreamPartEncoder<TFrame>>> }
impl<TFrame> BitStreamEncoder<TFrame> {
    pub fn new(parts: Vec<Box<dyn BitStreamPartEncoder<TFrame>>>) -> Self;
    pub fn run(&mut self, frame: &mut TFrame, bs: &mut BitStream); // C++ Do()
}
```

Implementation notes:
- C++ passes `void* frameData`; Rust uses a generic frame type parameter instead — each codec instantiates with its own frame struct. If trait objects over generics get awkward during codec porting, an acceptable fallback is one concrete encoder type per codec; decide when porting phase 08 (first heavy user).
- Read `encode.cpp` for: the exact binary-search loop (how lambda midpoint and termination are computed), `Repeat` semantics (restart from first stage), and the global-consumption accounting across stages. Port the logic faithfully.

### 3. `util` module

| C++ | Rust |
|---|---|
| `SwapArray(p, len)` | `slice.reverse()` at call sites, or `pub fn swap_array<T>(s: &mut [T])` wrapper to keep ported code 1:1 |
| `InvertSpectrInPlase<N>` / `InvertSpectr<N>` | `pub fn invert_spectr(s: &mut [f32])` — negate every even index |
| `GetFirstSetBit(u32)` | `pub fn get_first_set_bit(x: u32) -> u16` — implement as `31 - x.leading_zeros()` for x>0; verify against the De Bruijn table semantics incl. x==0 (C++ returns 0) |
| `Div8Ceil` | `pub fn div8_ceil(x: u32) -> u32 { 1 + (x - 1) / 8 }` — keep exact formula (note: C++ gives 1 for x=0 due to wrap? no — (0-1)/8 underflows; never called with 0; replicate formula with `x.wrapping_sub(1)` only if tests require; otherwise assert x>0) |
| `CalcMedian` | `pub fn calc_median(in_: &[f32]) -> f32` — sort copy, take `(len-1)/2` |
| `CalcEnergy` | `pub fn calc_energy(in_: &[f32]) -> f32` — note C++ accumulates into `0.0` (f64 init but T add); replicate as plain f32 sum of squares, tests are tolerance-based |
| `ToInt(float)` | `pub fn to_int(x: f32) -> i32 { x.round_ties_even() as i32 }` — matches `lrint` under FE_TONEAREST |

### 4. Tests to port

- **`bitstream_ut.cpp`** (134 L): write/read sequences crossing byte and 32-bit word boundaries; `MakeSign` cases. Port all cases as `#[test]` fns.
- **`encode_ut.cpp`** (178 L): exercises the bit-allocation binary search with mock part encoders hitting a target budget, incl. the `Repeat` path. Port the mock encoders as small structs implementing the trait.
- **`util_ut.cpp`** (52 L): `SwapArray`, `GetFirstSetBit`, `CalcEnergy` cases — port verbatim.

## Acceptance criteria

- `cargo test -p atracdenc-core` green with all ported cases.
- Bitstream round-trip property: for arbitrary (val, n≤32) sequences, write-then-read returns the same values (add one extra property-style test with a fixed seed).
- No `unsafe`, no panics on valid inputs.
