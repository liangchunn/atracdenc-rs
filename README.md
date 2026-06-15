# atracdenc-rs

A free LGPL implementation of ATRAC1, ATRAC3, and ATRAC3plus encoders, ported
from C++ to Rust.

> [!WARNING]
> The code in this repository are largely written and reviewed with the aid of AI LLMs.
>
> I have personally tested and inspected the outputs by comparing it to Sony's `a3tool` and the reference C++ `atracdenc` outputs, and tested uploads onto a real NetMD device with success.
>
> See comparisons in the `docs/` directory

**Original C++ reference:** <https://github.com/dcherednik/atracdenc>  
**Ported from commit:** [`01234b0`][upstream-commit] ("Add explicit container selection")

[upstream-commit]: https://github.com/dcherednik/atracdenc/commit/01234b0

## Crates

The workspace contains three crates:

| Crate                                      | Cargo package    | Type    | Audience                                                             |
| ------------------------------------------ | ---------------- | ------- | -------------------------------------------------------------------- |
| [`atracdenc-core`](crates/atracdenc-core/) | `atracdenc-core` | Library | Advanced users needing low-level codec, DSP, or container primitives |
| [`atracdenc`](crates/atracdenc/)           | `atracdenc`      | Library | Application developers; provides `EncodeBuilder` / `DecodeBuilder`   |
| [`atracdenc-cli`](crates/atracdenc-cli/)   | `atracdenc-cli`  | Binary  | End users running the `atracdenc` command-line tool                  |

### `atracdenc-core`

Low-level library with ATRAC1/ATRAC3/ATRAC3plus encode/decode primitives

### `atracdenc` (library)

High-level facade crate providing `EncodeBuilder` and `DecodeBuilder`. This is the **recommended
dependency** for applications that want to encode or decode ATRAC files
programmatically.

**Minimal example — ATRAC1 encode in memory:**

```rust
use atracdenc::{Codec, EncodeBuilder};

fn main() -> atracdenc::Result<()> {
    let wav = std::fs::read("input.wav")?;
    let aea = EncodeBuilder::new()
        .codec(Codec::Atrac1)
        .input_bytes(wav)
        .run_to_vec()?;
    std::fs::write("output.aea", aea)?;
    Ok(())
}
```

See [`crates/atracdenc/README.md`](crates/atracdenc/README.md) for examples and documentation.

### `atracdenc-cli` (binary)

CLI frontend that delegates to the `atracdenc` library for encoding or decoding.

> [!NOTE]
> The Cargo package `atracdenc` (in `crates/atracdenc/`) is a **library**.
>
> The Cargo package `atracdenc-cli` (in `crates/atracdenc-cli/`) produces the **`atracdenc` binary**.

```bash
cargo build --release -p atracdenc-cli    # produces target/release/atracdenc
```

## Input Constraints

Encoding input must be a **44100 Hz, 16-bit, mono or stereo WAV** file.
No resampling or format conversion is performed. Decode currently supports ATRAC1 from AEA input only.

These constraints apply to both the CLI and the `atracdenc` library crate.

## Port Status

| Codec                  | Encode | Decode |
| ---------------------- | ------ | ------ |
| **ATRAC1**             | Yes    | Yes    |
| **ATRAC3** (LP2 / LP4) | Yes    | No     |
| **ATRAC3plus**         | Yes    | No     |

Use `ffmpeg` for ATRAC3 and ATRAC3plus decode

## Containers

| Container  | Extension      | ATRAC1 | ATRAC3 | ATRAC3plus |
| ---------- | -------------- | ------ | ------ | ---------- |
| AEA        | `.aea`         | Yes    | —      | —          |
| OMA        | `.oma`, `.omg` | —      | Yes    | Yes        |
| RIFF/WAV   | `.at3`, `.wav` | —      | Yes    | Yes        |
| RealMedia  | `.rm`, `.ra`   | —      | Yes    | —          |
| Raw frames | `.raw`, `.dat` | Yes    | Yes    | Yes        |

The CLI infers the container from the output file extension; use `--container`
to override. The `atracdenc` library crate requires an explicit
`container(...)` call (see below).

## Building

Requires Rust >1.96.0 (edition 2024).

```bash
# build the binary into `target/release/atracdenc`.
cargo build --release -p atracdenc-cli
```

## CLI Usage

```bash
# show the help message
atracdenc --help

# ATRAC1 encode (WAV → AEA)
atracdenc -e atrac1 -i input.wav -o output.aea

# ATRAC1 decode (AEA → WAV)
atracdenc -e atrac1 -d -i input.aea -o output.wav

# ATRAC3 encode, LP2 stereo (~132 kbps)
atracdenc -e atrac3 -i input.wav -o output.oma

# ATRAC3 LP4 encode, joint stereo (~66 kbps)
atracdenc -e atrac3-lp4 -i input.wav -o output.oma

# ATRAC3 C++-parity BFU allocation (slower, closer to reference output)
atracdenc -e atrac3 --at3-bfu-mode parity -i input.wav -o output.oma

# ATRAC3plus encode (defaults to OMA container)
atracdenc -e atrac3plus -i input.wav -o output.oma

# Explicit container override
atracdenc -e atrac3 --container riff -i input.wav -o output.at3

# ATRAC3 aligned for Sony hardware playback (MiniDisc/NetMD): lag-0, exact length
atracdenc -e atrac3 --container riff --sony-delay-align -i input.wav -o output.at3

# Custom bitrate (ATRAC3 only, 32–384 kbps)
atracdenc -e atrac3 --bitrate 192 -i input.wav -o out.oma

# Customise logging levels
RUST_LOG=off atracdenc -e atrac3 -i input.wav -o output.oma
```

ATRAC3 uses fast BFU allocation by default; use `--at3-bfu-mode parity` when
comparing encoder output against the C++ reference. ATRAC3plus uses GHA-based
tonal analysis; pass `--advanced ghadbg=<mask>` to control GHA processing flags.

## Sony decode-delay alignment

By default (matching the C++ reference), ATRAC3 output decoded by Sony's
reference decoder — including MiniDisc / NetMD hardware and Sony's `at3tool` —
plays back 69 samples late and slightly clipped at the tail. This is the
ATRAC3 encoder delay, not a bug, and is inaudible for most uses.

The opt-in `--sony-delay-align` flag makes the output reproduce the original PCM
at **zero sample delay and exact length** through Sony's decoder, so
atracdenc-encoded tracks behave identically to Sony-encoded ones on hardware.
It is supported for the standard MiniDisc modes — **ATRAC3 LP2 and LP4, stereo,
RIFF/AT3** output — and leaves default (non-aligned) output byte-identical.

```bash
# LP2 (~132 kbps)
atracdenc -e atrac3 --container riff --sony-delay-align -i input.wav -o output.at3

# LP4 (~66 kbps)
atracdenc -e atrac3-lp4 --container riff --sony-delay-align -i input.wav -o output.at3
```

See [`docs/sony-delay-alignment.md`](docs/sony-delay-alignment.md) for the full
investigation, the reverse engineering of Sony's `psp_at3tool.exe`, and the
implementation details.

## Performance

This crate is generally faster than the C++ reference due to certain optimisations and dead-work elimination.

See [`speed-snr-comparison.md`](./docs/speed-snr-comparison.md).

## License

[LGPL-2.1-or-later](LICENSE), same as the original C++ project.

ATRAC, ATRAC3, ATRAC3plus, and their logos are trademarks of Sony Corporation.

## References

- Original C++ project: <https://github.com/dcherednik/atracdenc>
- ATRAC1 specification (Sony, 1994)
- ATRAC3 specification (Sony, 2000)
