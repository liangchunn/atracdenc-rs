# atracdenc

High-level Rust API for encoding and decoding ATRAC files.

This crate is the ergonomic facade over `atracdenc-core`: it accepts WAV bytes
or caller-owned readers/writers, validates the supported PCM format, selects
containers, and wires the codec encoder/decoder into the PCM engine. Use this
crate from applications; use `atracdenc-core` only when you need lower-level
codec or container primitives.

## Supported Input

Encoding input must be a WAV file with:

- 44100 Hz sample rate
- 16-bit PCM samples
- mono or stereo channels

The facade does not resample or convert PCM formats. Decode currently supports
ATRAC1 from AEA input only.

## Dependency

Inside this workspace:

```toml
[dependencies]
atracdenc = { path = "crates/atracdenc" }
```

From another crate in the same workspace, use the relative path to this package:

```toml
[dependencies]
atracdenc = { path = "../atracdenc" }
```

## Mode Summary

| Mode | API selection | Default container | Notes |
|---|---|---|---|
| ATRAC1 | `Codec::Atrac1` | AEA (`.aea`) | Encodes and decodes. |
| ATRAC3 LP2 | `Codec::Atrac3` with default `At3Settings` | OMA (`.oma`) | Default ATRAC3 mode, about 132 kbps stereo. |
| ATRAC3 LP4 | `Codec::Atrac3Lp4` | OMA (`.oma`) | Low-bitrate ATRAC3 mode, about 66 kbps stereo. |
| ATRAC3plus | `Codec::Atrac3plus` | OMA (`.oma`) | Encodes only. `--advanced ghadbg=<mask>` controls GHA flags. |

For ATRAC3, `bitrate_kbps` is a request. The encoder selects the first supported
ATRAC3 frame format at or above that requested rate.

## Encode to Memory

Use `input_bytes(...)` or `input_reader(...)` with `run_to_vec()` when the
application owns file loading and storage.

```rust
use atracdenc::{Codec, Container, EncodeBuilder};

fn main() -> atracdenc::Result<()> {
    let wav_bytes = std::fs::read("input.wav")?;
    let aea_bytes = EncodeBuilder::new()
        .codec(Codec::Atrac1)
        .input_bytes(wav_bytes)
        .container(Container::Aea)
        .run_to_vec()?;

    std::fs::write("output.aea", aea_bytes)?;
    Ok(())
}
```

If `container(...)` is omitted for in-memory output, ATRAC1 defaults to AEA,
ATRAC3 defaults to OMA, and ATRAC3plus defaults to OMA.

## Decode to Memory

ATRAC1 AEA decode can also return WAV bytes.

```rust
use atracdenc::{Codec, DecodeBuilder};

fn main() -> atracdenc::Result<()> {
    let aea_bytes = std::fs::read("input.aea")?;
    let wav_bytes = DecodeBuilder::new()
        .codec(Codec::Atrac1)
        .input_bytes(aea_bytes)
        .run_to_vec()?;

    std::fs::write("decoded.wav", wav_bytes)?;
    Ok(())
}
```

## Seekable Writers

Use `output_writer(...)` when you want the facade to write into a caller-owned
seekable sink such as `std::io::Cursor<Vec<u8>>`. Container outputs are modeled
as `Write + Seek` because some formats patch headers after encoding.

## Encode ATRAC1 to a Writer

```rust
use atracdenc::{Codec, Container, EncodeBuilder};
use std::{
    fs::File,
    io::{BufReader, BufWriter},
};

fn main() -> atracdenc::Result<()> {
    let input = BufReader::new(File::open("input.wav")?);
    let output = BufWriter::new(File::create("output.aea")?);

    EncodeBuilder::new()
        .codec(Codec::Atrac1)
        .input_reader(input)
        .output_writer(output)
        .container(Container::Aea)
        .run()
}
```

The facade does not open paths or infer containers from filenames. Applications
that use files should open their own readers/writers and set `container(...)`
when they need a specific container.

## Decode ATRAC1 to a Writer

```rust
use atracdenc::{Codec, DecodeBuilder};
use std::{
    fs::File,
    io::{BufReader, BufWriter},
};

fn main() -> atracdenc::Result<()> {
    let input = BufReader::new(File::open("input.aea")?);
    let output = BufWriter::new(File::create("decoded.wav")?);

    DecodeBuilder::new()
        .codec(Codec::Atrac1)
        .input_reader(input)
        .output_writer(output)
        .run()
}
```

## Encode ATRAC3 LP2

`Codec::Atrac3` defaults to the LP2-sized ATRAC3 frame format, about 132 kbps
for stereo input.

```rust
use atracdenc::{Codec, Container, EncodeBuilder};

fn main() -> atracdenc::Result<()> {
    let wav_bytes = std::fs::read("input.wav")?;
    let bytes = EncodeBuilder::new()
        .codec(Codec::Atrac3)
        .input_bytes(wav_bytes)
        .container(Container::Oma)
        .run_to_vec()?;

    std::fs::write("lp2.oma", bytes)?;
    Ok(())
}
```

## Encode ATRAC3 LP4 as RIFF

Use `Codec::Atrac3Lp4` for the low-bitrate ATRAC3 mode. This example writes a
RIFF/WAV-style ATRAC3 file and requests 64 kbps, which maps to the LP4 frame
format.

```rust
use atracdenc::{At3Settings, Codec, Container, EncodeBuilder};

fn main() -> atracdenc::Result<()> {
    let wav_bytes = std::fs::read("input.wav")?;
    let bytes = EncodeBuilder::new()
        .codec(Codec::Atrac3Lp4)
        .input_bytes(wav_bytes)
        .container(Container::Riff)
        .at3_settings(At3Settings {
            bitrate_kbps: Some(64),
            ..At3Settings::default()
        })
        .run_to_vec()?;

    std::fs::write("lp4.at3", bytes)?;
    Ok(())
}
```

## Sony decode-delay alignment (ATRAC3 LP2 / LP4)

Set `At3Settings::sony_delay_align` so the encoded stream reproduces the original
PCM at **zero sample delay and exact length** when decoded by Sony's reference
decoder (MiniDisc / NetMD hardware, or `at3tool`). Without it (the default,
C++-reference-compatible behavior), Sony's decoder reproduces the audio 69
samples late and slightly clipped at the tail — the ATRAC3 encoder delay.

This option is supported for the standard MiniDisc modes — **ATRAC3 LP2
(`Codec::Atrac3`) and LP4 (`Codec::Atrac3Lp4`), stereo, into the RIFF/AT3
container**. It returns an error for mono input, other containers, non-LP2/LP4
bitrates, or non-ATRAC3 codecs. Default output is unaffected. See
`docs/sony-delay-alignment.md` for the background.

```rust
use atracdenc::{At3Settings, Codec, Container, EncodeBuilder};

fn main() -> atracdenc::Result<()> {
    let wav_bytes = std::fs::read("input.wav")?; // 44.1 kHz / 16-bit / stereo
    let bytes = EncodeBuilder::new()
        .codec(Codec::Atrac3) // or Codec::Atrac3Lp4 for LP4
        .input_bytes(wav_bytes)
        .container(Container::Riff)
        .at3_settings(At3Settings {
            sony_delay_align: true,
            ..At3Settings::default()
        })
        .run_to_vec()?;

    std::fs::write("aligned.at3", bytes)?;
    Ok(())
}
```

## Containers

If `container(...)` is omitted, ATRAC1 defaults to AEA, ATRAC3 defaults to
OMA, and ATRAC3plus defaults to OMA. Filename extension inference is handled by
`atracdenc-cli`, not this facade crate.

Supported combinations:

| Codec | Containers |
|---|---|
| `Codec::Atrac1` | `Container::Aea`, `Container::Raw` |
| `Codec::Atrac3`, `Codec::Atrac3Lp4` | `Container::Oma`, `Container::Riff`, `Container::Rm`, `Container::Raw` |
| `Codec::Atrac3plus` | `Container::Oma`, `Container::Riff`, `Container::Raw` |
