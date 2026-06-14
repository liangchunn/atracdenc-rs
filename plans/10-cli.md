# 10 — CLI (`atracdenc` binary)

## Goal

Implement the `atracdenc-cli` crate: full command-line parity with the C++ binary — codec/container selection and validation, encode/decode wiring through the PCM engine, progress display, and exact-enough error messages for the ported integration tests.

## Prerequisites

Phases 06–09 (codecs + containers + engine). A scaffold handling only ATRAC1 can be built right after phase 07 to enable early end-to-end testing; finish after 09.

## C++ sources

| C++ file | Lines | Rust target |
|---|---|---|
| `atracdenc/src/main.cpp` | 757 | `atracdenc-cli/src/main.rs` (+ `cli.rs`, `progress.rs`) |
| `atracdenc/src/help.cpp/.h` | 46 | clap-generated help (match content, not formatting) |
| `atracdenc/man/atracdenc.1` | — | reference for option semantics |
| `atracdenc/src/platform/win/getopt/` | 655 | **not ported** |

## Option surface (must match `main.cpp` / man page)

| Option | Semantics |
|---|---|
| `-e, --encode <codec>` | `atrac1` (default), `atrac3`, `atrac3_lp4` (= atrac3 @ 66kbps joint stereo... read main.cpp: lp4 maps to the 64kbps-class preset), `atrac3plus` |
| `-d, --decode` | ATRAC1 only; error for others |
| `-i <file>` | input (WAV for encode; AEA for decode) |
| `-o <file>` | output path |
| `-h, --help` | help text |
| `--bitrate <n>` | ATRAC3 + RM container path; range 32–384 as validated in main.cpp |
| `--container <c>` | `aea`, `oma`, `riff`, `rm`, `raw`; if absent, inferred from output extension (`.aea`, `.oma`/`.omg`, `.at3`/`.wav`?, `.rm`, fallback — read `main.cpp` inference code for exact extension map) |
| `--bfuidxconst <n>` | ATRAC1: 1–8; also ATRAC3 (check range validation in main.cpp) |
| `--bfuidxfast` | deprecated no-op; accept and ignore (print the same deprecation notice if C++ does) |
| `--notransient[=mask]` | ATRAC1: disable transient detection; optional mask forces short windows per band |
| `--notonal` | ATRAC3: disable tonal components |
| `--nogaincontrol` | ATRAC3: disable gain control |
| `--advanced <opts>` | ATRAC3+: forwarded to `at3p::parse_advanced_opt` |
| `--yaml-log <file>` | ATRAC3: gain-control YAML debug log (phase 08) |
| `-m` | accepted in C++ optstring (vestigial mono flag) — accept-and-ignore for compat |

## Steps

### 1. Argument definition (`cli.rs`)

clap derive struct mirroring the table above. Notes:
- `--notransient` with *optional* value: clap `num_args(0..=1)` + `require_equals(true)` to support both `--notransient` and `--notransient=mask`.
- Keep clap's native help; transcribe the descriptive text from `help.cpp`/man page. Exact help-text parity is not required (no test depends on it).

### 2. Validation logic (port from `main.cpp`)

Port the decision tables:
- **Codec ↔ container validity matrix**: ATRAC1 → AEA/RAW; ATRAC3 → OMA/RIFF/RM/RAW; ATRAC3+ → OMA/RIFF/RAW. Reject invalid combos with messages equivalent to C++.
- **Container inference from output extension** when `--container` absent (read exact extension→container map in main.cpp, including default when extension unknown).
- **Input WAV constraints**: 44100 Hz, 16-bit; mirror C++ error text (integration tests may grep for it — cross-check with `test/integration/input_file_tests.py` expectations in phase 11).
- **Bitrate**: only meaningful for ATRAC3 (`params_for_bitrate`), validated range; RM container uses it for header fields.
- **Decode path**: only ATRAC1/AEA input; report errors for missing/invalid input files with messages matching what `input_file_tests.py` asserts (`missing-input` test).

### 3. Wiring (`main.rs`)

Port the `main.cpp` flow:
1. Parse args, open input (WAV reader for encode / AEA input for decode).
2. Build the container output (`aea/oma/at3/rm/raw` constructors from phase 06 with codec-specific params).
3. Construct the codec `Processor` (phases 07–09) with settings derived from flags (incl. `--yaml-log` file creation → `Box<dyn Write>` into the AT3 encoder).
4. Drive `PcmEngine::apply_process` in a loop with the codec's frame step (AT1: 512, AT3: 1024, AT3+: 2048 — confirm steps from each `GetLambda` usage in main.cpp), counting total samples for progress.
5. Treat `NoDataToRead` as normal EOF termination, flush/finalize containers (frame-count patching happens in container Drop/explicit `finalize()` — prefer explicit finalize to surface IO errors).
6. Exit codes: 0 success, nonzero with message on error — match C++ behavior (it returns 1 and prints to stderr; verify which stream messages go to, the integration tests check stderr/stdout).

### 4. Progress display (`progress.rs`)

Emit progress through Rust logging; users can suppress it with `RUST_LOG=off`.

### 5. Tests

- Unit tests for: container inference map, codec/container validity matrix, `--notransient=mask` parsing, bitrate validation. (Pure functions — test without process spawning.)
- Full CLI integration tests are phase 11.

## Acceptance criteria

- `cargo run -p atracdenc-cli -- -e atrac1 -i test.wav -o test.aea` produces a playable AEA; same for atrac3 → `.oma`, atrac3plus → `.oma`, atrac3 → `.rm`/`.at3`, and `-d` decodes AEA → WAV.
- `atracdenc --help` documents all options.
- Invalid combos rejected with clear messages; exit codes match C++ conventions.
