//! PCM I/O and the frame-pumping engine.
//!
//! [`engine`] hosts the [`engine::PcmEngine`] that pulls PCM frames through a
//! [`engine::Processor`] (an encoder/decoder) via [`engine::PcmReader`]/
//! [`engine::PcmWriter`]. [`wav`] provides the [`wav::WavReader`]/[`wav::WavWriter`].

pub mod engine;
pub mod wav;
