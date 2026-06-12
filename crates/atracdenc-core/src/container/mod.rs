use std::io;

pub mod aea;
pub mod at3;
pub mod oma;
pub mod raw;
pub mod rm;

pub trait CompressedOutput {
    fn write_frame(&mut self, data: &[u8]) -> io::Result<()>;
    fn name(&self) -> &str;
    fn channels(&self) -> usize;
}

pub trait CompressedInput {
    fn read_frame(&mut self) -> io::Result<Option<Vec<u8>>>;
    fn frame_size(&self) -> usize;
    fn length_in_samples(&self) -> u64;
    fn name(&self) -> &str;
    fn channels(&self) -> usize;
}
