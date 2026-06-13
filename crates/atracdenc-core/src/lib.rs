//! Core library for the Rust port of AtracDEnc.
//!
//! Derived from AtracDEnc by Daniil Cherednik and distributed under
//! LGPL-2.1-or-later. Some reference algorithms and tables in later phases
//! originate from FFmpeg-derived code in the C++ project.

pub mod at1;
pub mod at3;
pub mod atrac;
pub mod bitstream;
pub mod container;
pub mod dsp;
pub mod error;
pub mod pcm;
pub mod util;

pub use error::AtracdencError;
