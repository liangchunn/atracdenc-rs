//! DSP primitives shared across the codecs.
//!
//! Transforms ([`dct`], [`mdct`], [`fft`]), the [`qmf`] analysis/synthesis
//! filter banks, [`gain`] control, [`transient`] detection, [`delay_buffer`]
//! overlap state, and the [`upsampler`].

pub mod dct;
pub mod delay_buffer;
pub mod fft;
pub mod gain;
pub mod mdct;
pub mod qmf;
pub mod transient;
pub mod upsampler;
