//! Low-level ATRAC codec primitives for the Rust port of AtracDEnc.
//!
//! Derived from AtracDEnc by Daniil Cherednik and distributed under
//! LGPL-2.1-or-later. Some reference algorithms and tables in later phases
//! originate from FFmpeg-derived code in the C++ project.
//!
//! # Most users want [`atracdenc`] instead
//!
//! This crate is the codec engine. **Prefer the high-level [`atracdenc`] facade
//! crate** for encoding/decoding: it provides the `EncodeBuilder`/`DecodeBuilder`
//! API, WAV/codec/container validation, and container inference. Depend on
//! `atracdenc-core` directly only when you need to drive the codec, container,
//! DSP, or bitstream primitives yourself.
//!
//! [`atracdenc`]: https://docs.rs/atracdenc
//!
//! # Module map
//!
//! Codec front-ends:
//!
//! - [`at1`] — ATRAC1 encode/decode ([`at1::codec`]) plus its QMF, MDCT, bit
//!   allocation, and dequantiser stages.
//! - [`at3`] — ATRAC3 encode ([`at3::encoder`]), bitstream, MDCT, QMF.
//! - [`at3p`] — ATRAC3+ encode ([`at3p::encoder`]) and its FFmpeg-derived DSP.
//!
//! Shared building blocks:
//!
//! - [`atrac`] — shared psychoacoustics (psy model, scaling).
//! - [`dsp`] — DCT/MDCT/FFT, QMF, gain, transient detection, delay buffers.
//! - [`bitstream`] — bit-level reader/writer used by the codec bitstreams.
//! - [`container`] — AEA / OMA / RIFF(AT3) / RealMedia / Raw I/O.
//! - [`pcm`] — PCM engine and WAV I/O ([`pcm::engine`], [`pcm::wav`]).
//! - [`util`] — small spectral/bit helpers shared across modules.
//! - [`error`] — the crate error type [`AtracdencError`].

pub mod at1;
pub mod at3;
pub mod at3p;
pub mod atrac;
pub mod bitstream;
pub mod container;
pub mod dsp;
pub mod error;
pub mod gha;
pub mod pcm;
pub mod util;

pub use error::AtracdencError;
