//! ATRAC3+ encoder.
//!
//! Port of `atracdenc/src/atrac/at3p/` (Daniil Cherednik, LGPL-2.1-or-later)
//! with FFmpeg-derived tables/DSP (also LGPL-2.1-or-later).
//!
//! These modules are deliberately faithful, line-for-line ports of the C/C++
//! reference (validated against the upstream unit-test vectors). Several Clippy
//! style lints are allowed module-wide because rewriting the index loops,
//! explicit `dim * 0/1/2` matrix offsets, and manual copies/swaps would obscure
//! the correspondence with the reference and risk regressions in code that is
//! verified bit/index-exact by tests.
#![allow(
    clippy::needless_range_loop,
    clippy::manual_memcpy,
    clippy::manual_swap,
    clippy::identity_op,
    clippy::erasing_op,
    clippy::collapsible_if,
    clippy::implicit_saturating_sub,
    clippy::derivable_impls,
    clippy::too_many_arguments,
    clippy::excessive_precision
)]

pub mod bitstream;
pub mod encoder;
pub mod ff_dsp;
pub mod ff_tables;
pub mod gha;
pub mod mdct;
pub mod pqf;
pub mod tables;
