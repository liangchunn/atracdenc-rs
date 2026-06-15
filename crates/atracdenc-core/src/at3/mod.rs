//! ATRAC3 encoder.
//!
//! [`encoder`] holds the public [`encoder::Atrac3Encoder`]; [`qmf`] and [`mdct`]
//! are its transform stages and [`bitstream`] packs the encoded frames. [`data`]
//! carries the constants, bitrate tables, and [`data::EncodeSettings`].
//! [`yaml_log`] is an optional diagnostic trace of the encode.

pub mod bitstream;
pub mod data;
pub mod encoder;
pub mod mdct;
pub mod qmf;
pub mod yaml_log;
