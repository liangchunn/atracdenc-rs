# 01 — Workspace setup

## Goal

Replace the placeholder single-package Cargo project with a two-crate workspace: `atracdenc-core` (library) and `atracdenc-cli` (binary named `atracdenc`).

## Prerequisites

None. First phase.

## Current state

- Root `Cargo.toml` is a placeholder package (`name = "atracdenc"`, edition 2024) with a stub `src/lib.rs` (`add()` function). Both get replaced.
- `target/` and `Cargo.lock` exist from the placeholder build.

## Steps

1. **Convert root `Cargo.toml` to a virtual workspace manifest:**

   ```toml
   [workspace]
   resolver = "3"
   members = ["crates/atracdenc-core", "crates/atracdenc-cli"]

   [workspace.package]
   version = "0.1.0"
   edition = "2024"
   license = "LGPL-2.1-or-later"
   repository = ""        # fill if/when published

   [workspace.dependencies]
   rustfft = "6"
   hound = "3"
   clap = { version = "4", features = ["derive"] }
   ```

   Pin exact minor versions at implementation time (`cargo add` will pick current).

2. **Delete the placeholder `src/` directory** at the workspace root (the stub `lib.rs`).

3. **Create `crates/atracdenc-core`:**
   - `Cargo.toml`: `name = "atracdenc-core"`, `version/edition/license.workspace = true`; deps: `rustfft` (workspace). `hound` is added in phase 06.
   - `src/lib.rs` with the module skeleton (all stubs initially commented out or empty; modules get filled in by later phases):

     ```rust
     pub mod bitstream;   // phase 02
     pub mod util;        // phase 02
     pub mod dsp;         // phases 03–04
     pub mod atrac;       // phase 05
     pub mod pcm;         // phase 06
     pub mod container;   // phase 06
     pub mod at1;         // phase 07
     pub mod at3;         // phase 08
     pub mod gha;         // phase 09
     pub mod at3p;        // phase 09
     ```

     Only declare modules as they are created — start with an empty `lib.rs` plus a crate-level doc comment stating origin and license (LGPL-2.1-or-later, derived from atracdenc by Daniil Cherednik, parts derived from FFmpeg).

4. **Create `crates/atracdenc-cli`:**
   - `Cargo.toml`:

     ```toml
     [package]
     name = "atracdenc-cli"
     # version/edition/license from workspace

     [[bin]]
     name = "atracdenc"
     path = "src/main.rs"

     [dependencies]
     atracdenc-core = { path = "../atracdenc-core" }
     clap = { workspace = true }
     ```

   - `src/main.rs`: minimal stub (`fn main() {}`) until phase 10.

5. **`.gitignore`**: ensure `/target` is ignored (already present), keep `Cargo.lock` committed (workspace with a binary).

6. **README note** (optional, root `README.md`): one paragraph — Rust port of atracdenc, reference C++ in `./atracdenc/`, LGPL-2.1-or-later.

7. **License file**: copy `atracdenc/LICENSE` (LGPL 2.1) to workspace root as `LICENSE`.

## Acceptance criteria

- `cargo build` and `cargo test` succeed at the workspace root (no tests yet, both crates compile).
- `cargo run -p atracdenc-cli` runs the stub binary named `atracdenc`.
- `cargo metadata` shows exactly two workspace members.
