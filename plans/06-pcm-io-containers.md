# 06 — PCM engine, WAV IO, containers (AEA, OMA, RIFF/AT3, RM, RAW)

## Goal

Port the data-plumbing layer: the PCM pumping engine and its traits, WAV PCM read/write via `hound`, and all compressed-stream containers. After this phase a codec only needs to implement the processor trait and a frame writer to be end-to-end usable.

## Prerequisites

Phase 02. Add `hound` to `atracdenc-core`.

## C++ sources

| C++ file | Lines | Rust target |
|---|---|---|
| `atracdenc/src/pcmengin.h` | 199 | `atracdenc-core/src/pcm/engine.rs` |
| `atracdenc/src/wav.{h,cpp}` + `pcm_io_sndfile.cpp` | 305 | `atracdenc-core/src/pcm/wav.rs` (hound) |
| `atracdenc/src/compressed_io.h` | 63 | `atracdenc-core/src/container/mod.rs` |
| `atracdenc/src/aea.{h,cpp}` | 245 | `atracdenc-core/src/container/aea.rs` |
| `atracdenc/src/oma.{h,cpp}` + `lib/liboma/` (oma.h 78, liboma.c 362) | 534 | `atracdenc-core/src/container/oma.rs` (full reimplementation, liboma absorbed) |
| `atracdenc/src/at3.{h,cpp}` | 407 | `atracdenc-core/src/container/at3.rs` |
| `atracdenc/src/rm.{h,cpp}` | 307 | `atracdenc-core/src/container/rm.rs` |
| `atracdenc/src/raw.{h,cpp}` | 97 | `atracdenc-core/src/container/raw.rs` |
| `atracdenc/src/file.h`, `utf8_file.h` | 109 | **not ported** (std::fs; `File::sync_all` where C++ fsynced) |
| `atracdenc/src/platform/win/pcm_io/*` | ~1440 | **not ported** (hound) |

## Steps

### 1. `pcm/engine.rs` — PCM engine

C++: `TPCMBuffer` (interleaved f32), `IPCMReader/IPCMWriter`, `TPCMEngine::ApplyProcess(step, lambda)` with LOOK_AHEAD/PROCESSED draining logic, `IProcessor::GetLambda`.

Rust design:

```rust
pub enum ProcessResult { LookAhead, Processed }

#[derive(thiserror-style enums or manual)]  // TNoDataToRead, TPCMBufferTooSmall, TWrongReadBuffer, TEndOfRead
pub enum PcmEngineError { NoDataToRead, BufferTooSmall, WrongReadBuffer }

pub struct PcmBuffer { buf: Vec<f32>, num_channels: usize }
impl PcmBuffer {
    pub fn new(buf_size: u16, num_channels: usize) -> Self;
    pub fn size(&self) -> usize;                       // frames
    pub fn frame(&self, pos: usize) -> &[f32];         // C++ operator[]; panic on OOB like C++ abort
    pub fn frame_mut(&mut self, pos: usize) -> &mut [f32];
    pub fn channels(&self) -> u16;
    pub fn zero(&mut self, pos: usize, len: usize);
}

pub trait PcmReader { fn read(&mut self, data: &mut PcmBuffer, size: u32) -> bool; }
pub trait PcmWriter { fn write(&mut self, data: &PcmBuffer, size: u32); }

pub struct ProcessMeta { pub channels: u16 }

pub struct PcmEngine { buffer: PcmBuffer, writer: Option<Box<dyn PcmWriter>>,
                       reader: Option<Box<dyn PcmReader>>, processed: u64, to_drain: u64 }
impl PcmEngine {
    pub fn new(buf_size: u16, num_channels: usize,
               reader: Option<Box<dyn PcmReader>>, writer: Option<Box<dyn PcmWriter>>) -> Self;
    pub fn apply_process<F>(&mut self, step: usize, lambda: &mut F) -> Result<u64, PcmEngineError>
        where F: FnMut(&mut [f32], &ProcessMeta) -> ProcessResult;
}

pub trait Processor {
    fn process_frame(&mut self, data: &mut [f32], meta: &ProcessMeta) -> ProcessResult;
}
```

- Port `ApplyProcess` (pcmengin.h lines 152–192) faithfully: read whole buffer, iterate by `step`, LOOK_AHEAD increments `to_drain`, drain mode breaks after `to_drain--`, partial write of `last_pos` frames, returns cumulative `processed`.
- C++ `IProcessor::GetLambda` returns a stateful closure; in Rust make codecs implement `Processor` (method instead of closure — avoids `FnMut` boxing gymnastics). The engine takes `&mut dyn Processor` or the closure form above; pick the trait form and adapt `apply_process` accordingly.
- C++ throws `TNoDataToRead` to signal completion to the main loop — in Rust return `Err(NoDataToRead)` and let the CLI loop treat it as EOF (not a failure).

### 2. `pcm/wav.rs` — WAV IO via hound

C++ `TWav`/`TWavPcmReader/Writer` over libsndfile. Rust:

```rust
pub struct WavReader { inner: hound::WavReader<BufReader<File>>, /* cached spec */ }
impl WavReader {
    pub fn open(path: &Path) -> Result<Self, WavError>;
    pub fn channels(&self) -> u16;
    pub fn sample_rate(&self) -> u32;
    pub fn total_samples(&self) -> u64;     // per channel (C++ GetTotalSamples)
    // implements PcmReader: fills PcmBuffer with f32 in [-1, 1), zero-pads tail, returns false at EOF
}
pub struct WavWriter { ... } // 16-bit PCM out; implements PcmWriter; finalize() on drop or explicit
```

- Conversion: C++ libsndfile `readf_float` yields floats normalized by 1/32768; replicate exactly (`sample as f32 / 32768.0`), and on write clamp + `to_int`-round to i16 the same way `pcm_io_sndfile.cpp` does (read it for the clipping behavior).
- Validation stays in the CLI (44100 Hz, 16-bit), but expose `sample_rate()`/`bits_per_sample` so the CLI can enforce it with the same error messages.

### 3. `container/mod.rs` — traits

```rust
pub trait CompressedOutput {
    fn write_frame(&mut self, data: &[u8]);
    fn name(&self) -> &str;
    fn channels(&self) -> usize;
}
pub trait CompressedInput {
    fn read_frame(&mut self) -> Option<Vec<u8>>;   // None at EOF
    fn frame_size(&self) -> usize;
    fn length_in_samples(&self) -> u64;
    fn name(&self) -> &str;
    fn channels(&self) -> usize;
}
```

IO errors: C++ throws; Rust containers should return `Result` — wrap methods in `Result<_, io::Error>` (adjust trait accordingly; keep it simple: `anyhow`-free, plain `std::io::Error` or a small `ContainerError`).

### 4. `container/aea.rs` — AEA (MiniDisc ATRAC1), read + write

Port `aea.cpp`: 2048-byte header (magic, 16-byte title, u32 frame count, channel count byte), 212-byte frames per channel, the dummy-frame prefix on encode, and frame-count patching on close (seek back + rewrite). Implements **both** `CompressedOutput` (encode) and `CompressedInput` (decode — ATRAC1 decoder reads AEA). Read `aea.cpp` for exact header field offsets and the first-frames skip logic.

### 5. `container/oma.rs` — OMA write (absorbs liboma)

Reimplement from `lib/liboma/src/liboma.c` + `src/oma.cpp` (write path only — the encoder never reads OMA):
- EA3 header (96 bytes: "ea3" tag v2 header then "EA3" 96-byte block), codec IDs `OMAC_ID_ATRAC3 = 0`, `OMAC_ID_ATRAC3PLUS = 1`, framesize/bitrate/joint-stereo encoding into the 4-byte codec params (read `oma_write_header`/param packing in liboma.c).
- `OmaWriter::new(path, codec: OmaCodec, channels, framesize, jointstereo)` mirroring `TOma`'s constructor args.

### 6. `container/at3.rs` — RIFF/WAVE AT3 + AT3+ write

Port `at3.cpp` (378 L): RIFF header with `WAVE_FORMAT_EXTENSIBLE`-style fmt chunks — ATRAC3 (`0x0270` fmt tag with extra codec data) and ATRAC3+ (GUID-based extensible fmt), `fact` chunk with sample length, `data` chunk size patched on finalize. Two constructors mirroring `CreateAt3Output` / `CreateAt3POutput` (read the cpp for exact field values: block align, samples per frame, flags). All multi-byte fields little-endian (`to_le_bytes`).

### 7. `container/rm.rs` — RealMedia write

Port `rm.cpp` (283 L): `.RMF`, `PROP`, `MDPR` (with ATRAC3-specific opaque data incl. "ra5" sub-header), `DATA` chunks; per-frame packet headers; header fields patched on finalize (num packets, data size). All big-endian (`to_be_bytes`). Mirrors `CreateRmOutput(bitrate, ...)`.

### 8. `container/raw.rs`

Port `raw.cpp`: headerless frame dump, optional fixed frame size with zero padding (used with `--container raw`).

### 9. Tests (new — C++ has no container UTs)

- AEA: write N frames → read back with `AeaInput`; header fields, frame count patch, title truncation round-trip.
- OMA/AT3/RM: golden-byte tests — write a tiny stream (2–3 dummy frames) to memory/temp file and assert exact header bytes against fixtures captured from the C++ implementation (generate once by running the C++ binary on a tiny input, or hand-derive from the spec in the code). At minimum assert: magic bytes, sizes, endianness of patched fields.
- WAV: hound round-trip 44.1k/16-bit; normalization matches `sample/32768.0`.
- Engine: LOOK_AHEAD/drain logic test — a fake processor that needs 1 frame of look-ahead; verify total processed count and writer-received frames match the C++ semantics.

## Acceptance criteria

- All new tests green.
- A dummy "passthrough" processor pumped through `PcmEngine` with WAV in/out reproduces the input WAV byte-exactly (sans optional header padding differences).
- Container outputs for fixture frames byte-identical to C++-produced references (headers; frame payloads are caller-provided so trivially identical).
