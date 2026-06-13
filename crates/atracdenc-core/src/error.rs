use std::io;

use crate::{container::ContainerError, pcm::engine::PcmEngineError};

#[derive(Debug, thiserror::Error)]
pub enum AtracdencError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("PCM engine error: {0}")]
    PcmEngine(#[from] PcmEngineError),
    #[error("container error: {0}")]
    Container(#[from] ContainerError),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}
