# atracdenc (Rust port)

A free LGPL implementation of ATRAC1 and ATRAC3 encoders, ported from C++ to
Rust.

**Original C++ reference:** <https://github.com/dcherednik/atracdenc>  
**Ported from commit:** [`01234b0`][upstream-commit] ("Add explicit container selection")

[upstream-commit]: https://github.com/dcherednik/atracdenc/commit/01234b0

## Crates

| Crate | Type | Description |
|---|---|---|
| [`atracdenc-core`](crates/atracdenc-core/) | Library | ATRAC1 and ATRAC3 encode/decode engine |
| [`atracdenc-cli`](crates/atracdenc-cli/) | Binary | CLI frontend (produces the `atracdenc` binary) |

The workspace is a virtual manifest — build the CLI to get the binary:

```bash
cargo build --release -p atracdenc-cli
```

## Port Status

| Codec | Encode | Decode |
|---|---|---|
| **ATRAC1** | Yes | Yes |
| **ATRAC3** (LP2 / LP4) | Yes | No |
| **ATRAC3plus** | **Not ported** | No |

ATRAC3plus encoding is not yet ported. The CLI will report an error if
`--encode atrac3plus` is requested.

## Containers

| Container | Extension | ATRAC1 | ATRAC3 |
|---|---|---|---|
| AEA | `.aea` | Yes | — |
| OMA | `.oma`, `.omg` | — | Yes |
| RIFF/WAV | `.at3`, `.wav` | — | Yes |
| RealMedia | `.rm`, `.ra` | — | Yes |
| Raw frames | `.raw` | Yes | Yes |

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

# ATRAC3 LP4 encode, joint stereo (~66 kbps)
atracdenc -e atrac3-lp4 -i input.wav -o output.oma

# Explicit container override
atracdenc -e atrac3 --container riff -i input.wav -o output.at3

# Custom bitrate (ATRAC3 only, 32–384 kbps)
atracdenc -e atrac3 --bitrate 192 -i input.wav -o out.oma

# Disable ATRAC3 gain control
atracdenc -e atrac3 --nogaincontrol -i input.wav -o out.oma

# Gain-control debug log (ATRAC3 only)
atracdenc -e atrac3 --yaml-log gain.yaml -i input.wav -o out.oma

# Disable ATRAC1 transient detection per band
atracdenc -e atrac1 --notransient 0xff -i in.wav -o out.aea

# Suppress progress output
atracdenc -e atrac3 --nostdout -i input.wav -o output.oma
```

Input must be 44100 Hz, 16-bit, mono or stereo WAV. Decode only supports ATRAC1
from AEA input.

## Building

Requires Rust 1.85+ (edition 2024).

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
