use std::io;

use crate::{
    bitstream::encode::BitStreamEncodeError, container::ContainerError, pcm::engine::PcmEngineError,
};

#[derive(Debug, thiserror::Error)]
pub enum AtracdencError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("PCM engine error: {0}")]
    PcmEngine(#[from] PcmEngineError),
    #[error("container error: {0}")]
    Container(#[from] ContainerError),
    #[error("bitstream encode error: {0}")]
    BitStreamEncode(#[from] BitStreamEncodeError),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}
