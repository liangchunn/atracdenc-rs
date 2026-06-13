# AGENTS.md — atracdenc

## Build

```bash
cargo build --release -p atracdenc-cli    # production binary → target/release/atracdenc
```

Requires Rust ≥ 1.85 (edition 2024 workspace). No Makefile, no CI, no task runner.

## Test

```bash
cargo test                                 # everything (unit + integration + doc)
cargo test -p atracdenc-core               # core only, all tests
cargo test -p atracdenc-cli                # CLI only (unit + integration)
cargo test --test at1_roundtrip            # single integration test in core
cargo test --test integration              # CLI integration tests
```

Unit tests are inline `#[cfg(test)] mod tests` in nearly every source file.
Integration tests live under `crates/*/tests/`. No test profile overrides exist;
tests run in default `test` profile.

## Format / Lint

- No `rustfmt.toml` or `clippy.toml` — pure rustfmt/clippy defaults.
- Some files have minor fmt diffs. Run `cargo fmt` before committing.

## Architecture

Virtual workspace with two crates:

| Crate | Path | Role |
|---|---|---|
| `atracdenc-core` | `crates/atracdenc-core/` | Library: ATRAC1 encode/decode + ATRAC3 encode |
| `atracdenc-cli` | `crates/atracdenc-cli/` | Binary `atracdenc` wrapping core via clap |

Top-level module map of core (`lib.rs`):
`at1` `at3` `atrac` `bitstream` `container` `dsp` `error` `pcm` `util`

Shared psychoacoustics live in `atrac/` (psy, scale). Codec-specific code in `at1/` and `at3/`.
Containers (AEA, OMA, RIFF/WAV, RealMedia, Raw) in `container/`.
DSP primitives (DCT, MDCT, FFT, QMF, transient detection) in `dsp/`.
PCM engine + WAV I/O in `pcm/`.

This is a port of the C++ project at `https://github.com/dcherednik/atracdenc`
(commit `01234b0`). Plans in `plans/` document the porting strategy.

## Input constraints

CLI input must be **44100 Hz, 16-bit, mono or stereo WAV**. No resampling is performed.
ATRAC3 decode is not yet implemented (only ATRAC1 decode support).

## Test data

`*.wav` and `*.aea` files in the repo root are gitignored. Integration tests
generate synthetic WAVs in temp dirs — no committed test fixtures needed.

## Cross-validation

Cross-encoder validation against the C++ `atracdenc` reference binary uses
`docs/bench.sh` (requires hyperfine, ffmpeg, and a C++ atracdenc build).
`docs/compute_snr.py` computes PCM SNR between two WAVs.

Additional analysis notes in `docs/`:
`cpp-rust-parity-audit.md`, `decode-profiling.md`, `precision-analysis.md`,
`speed-snr-comparison.md`. Consult these when investigating encoder parity or
performance issues.

## Profiling

A `[profile.profiling]` workspace profile exists (release + debug symbols, no strip)
for sampling profilers like samply:

```bash
cargo build --profile profiling -p atracdenc-cli
```
