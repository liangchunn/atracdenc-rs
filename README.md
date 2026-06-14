# atracdenc (Rust port)

A free LGPL implementation of ATRAC1, ATRAC3, and ATRAC3plus encoders, ported
from C++ to Rust.

ATRAC, ATRAC3, ATRAC3plus, and their logos are trademarks of Sony Corporation.

**Original C++ reference:** <https://github.com/dcherednik/atracdenc>  
**Ported from commit:** [`01234b0`][upstream-commit] ("Add explicit container selection")

[upstream-commit]: https://github.com/dcherednik/atracdenc/commit/01234b0

## Crates

| Crate | Type | Description |
|---|---|---|
| [`atracdenc`](crates/atracdenc/) | Library | High-level facade: builders, validation, container inference |
| [`atracdenc-core`](crates/atracdenc-core/) | Library | ATRAC1, ATRAC3, and ATRAC3plus encode/decode engine |
| [`atracdenc-cli`](crates/atracdenc-cli/) | Binary | CLI frontend (produces the `atracdenc` binary) |

See [`crates/atracdenc/README.md`](crates/atracdenc/README.md) for library API
examples covering ATRAC1, ATRAC3 LP2, ATRAC3 LP4, and ATRAC3plus usage.

The workspace is a virtual manifest — build the CLI to get the binary:

```bash
cargo build --release -p atracdenc-cli
```

## Port Status

| Codec | Encode | Decode |
|---|---|---|
| **ATRAC1** | Yes | Yes |
| **ATRAC3** (LP2 / LP4) | Yes | No |
| **ATRAC3plus** | Yes | No |

ATRAC3plus decode is not yet ported.

## Containers

| Container | Extension | ATRAC1 | ATRAC3 | ATRAC3plus |
|---|---|---|---|---|
| AEA | `.aea` | Yes | — | — |
| OMA | `.oma`, `.omg` | — | Yes | Yes |
| RIFF/WAV | `.at3`, `.wav` | — | Yes | Yes |
| RealMedia | `.rm`, `.ra` | — | Yes | — |
| Raw frames | `.raw`, `.dat` | Yes | Yes | Yes |

Containers are inferred from the output file extension. Use `--container` to
override.

## CLI Usage

```bash
# ATRAC1 encode (WAV → AEA)
atracdenc -e atrac1 -i input.wav -o output.aea

# ATRAC1 decode (AEA → WAV)
atracdenc -e atrac1 -d -i input.aea -o output.wav

# ATRAC3 encode, LP2 stereo (~132 kbps)
atracdenc -e atrac3 -i input.wav -o output.oma

# ATRAC3 C++-parity BFU allocation (slower, closer to reference output)
atracdenc -e atrac3 --at3-bfu-mode parity -i input.wav -o output.oma

# ATRAC3 LP4 encode, joint stereo (~66 kbps)
atracdenc -e atrac3-lp4 -i input.wav -o output.oma

# ATRAC3plus encode (defaults to OMA container)
atracdenc -e atrac3plus -i input.wav -o output.oma

# ATRAC3plus with GHA debug flags
atracdenc -e atrac3plus --advanced ghadbg=0 -i input.wav -o output.oma

# Explicit container override
atracdenc -e atrac3 --container riff -i input.wav -o output.at3

# ATRAC3 aligned for Sony hardware playback (MiniDisc/NetMD): lag-0, exact length
atracdenc -e atrac3 --container riff --sony-delay-align -i input.wav -o output.at3

# Custom bitrate (ATRAC3 only, 32–384 kbps)
atracdenc -e atrac3 --bitrate 192 -i input.wav -o out.oma

# Disable ATRAC3 gain control
atracdenc -e atrac3 --nogaincontrol -i input.wav -o out.oma

# Gain-control debug log (ATRAC3 only)
atracdenc -e atrac3 --yaml-log gain.yaml -i input.wav -o out.oma

# Disable ATRAC1 transient detection per band
atracdenc -e atrac1 --notransient 0xff -i in.wav -o out.aea

# Suppress log output
RUST_LOG=off atracdenc -e atrac3 -i input.wav -o output.oma
```

Input must be 44100 Hz, 16-bit, mono or stereo WAV. Decode only supports ATRAC1
from AEA input. ATRAC3 uses fast BFU allocation by default; use
`--at3-bfu-mode parity` when comparing encoder output against the C++
reference. ATRAC3plus uses GHA-based tonal analysis; pass
`--advanced ghadbg=<mask>` to control GHA processing flags.

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


## Building

Requires Rust 1.96.0 (edition 2024).

```bash
cargo build --release -p atracdenc-cli
```

The binary will be at `target/release/atracdenc`.

## License

[LGPL-2.1-or-later](LICENSE), same as the original C++ project.

## References

- Original C++ project: <https://github.com/dcherednik/atracdenc>
- ATRAC1 specification (Sony, 1994)
- ATRAC3 specification (Sony, 2000)
