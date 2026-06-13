use std::io::Write;

use super::{CompressedOutput, ContainerError};

pub struct RawOutput<W: Write> {
    inner: W,
    num_channels: usize,
    frame_size: Option<usize>,
}

impl<W: Write> RawOutput<W> {
    pub fn new(inner: W, num_channels: usize, frame_size: Option<usize>) -> Self {
        Self {
            inner,
            num_channels,
            frame_size,
        }
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> CompressedOutput for RawOutput<W> {
    fn write_frame(&mut self, data: &[u8]) -> Result<(), ContainerError> {
        if let Some(frame_size) = self.frame_size {
            let mut frame = data.to_vec();
            frame.resize(frame_size, 0);
            self.inner.write_all(&frame)?;
        } else {
            self.inner.write_all(data)?;
        }
        Ok(())
    }

    fn name(&self) -> &str {
        ""
    }

    fn channels(&self) -> usize {
        self.num_channels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_output_pads_to_fixed_frame_size() {
        let mut out = RawOutput::new(Vec::new(), 2, Some(4));
        out.write_frame(&[1, 2]).unwrap();
        out.write_frame(&[3, 4, 5, 6, 7]).unwrap();
        assert_eq!(vec![1, 2, 0, 0, 3, 4, 5, 6], out.into_inner());
    }
}
