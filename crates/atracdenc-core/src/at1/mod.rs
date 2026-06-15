//! ATRAC1 codec.
//!
//! [`codec`] holds the public [`codec::Atrac1Encoder`]/[`codec::Atrac1Decoder`];
//! the remaining modules are the encode/decode stages: [`qmf`] (band split),
//! [`mdct`] (per-band transform), [`bitalloc`] (bit allocation), and
//! [`dequantiser`] (decode-side spectrum reconstruction). [`data`] carries the
//! shared constants, tables, and [`data::EncodeSettings`].

pub mod bitalloc;
pub mod codec;
pub mod data;
pub mod dequantiser;
pub mod mdct;
pub mod qmf;
