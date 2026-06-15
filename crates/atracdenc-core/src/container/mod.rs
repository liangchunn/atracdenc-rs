//! Compressed-stream container I/O.
//!
//! Readers/writers for the supported containers: [`aea`] (ATRAC1 AEA),
//! [`oma`] (Sony OpenMG), [`at3`] (RIFF/WAV with AT3 fmt), [`rm`] (RealMedia),
//! and [`raw`] (headerless frames). The [`CompressedInput`]/[`CompressedOutput`]
//! traits abstract over them; [`ContainerError`] is the shared error type.

use std::io;

pub mod aea;
pub mod at3;
pub mod oma;
pub mod raw;
pub mod rm;

#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("container already consumed")]
    AlreadyConsumed,
    #[error("unsupported sample rate")]
    UnsupportedSampleRate,
}

pub trait CompressedOutput {
    fn write_frame(&mut self, data: &[u8]) -> Result<(), ContainerError>;
    fn name(&self) -> &str;
    fn channels(&self) -> usize;
}

pub trait CompressedInput {
    fn read_frame(&mut self) -> Result<Option<Vec<u8>>, ContainerError>;
    fn frame_size(&self) -> usize;
    fn length_in_samples(&self) -> u64;
    fn name(&self) -> &str;
    fn channels(&self) -> usize;
}
