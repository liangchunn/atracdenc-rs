#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ProcessResult {
    LookAhead,
    Processed,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PcmEngineError {
    NoDataToRead,
    BufferTooSmall,
    WrongReadBuffer,
}

#[derive(Debug, Clone)]
pub struct PcmBuffer {
    buf: Vec<f32>,
    num_channels: usize,
}

impl PcmBuffer {
    pub fn new(buf_size: u16, num_channels: usize) -> Self {
        Self {
            buf: vec![0.0; buf_size as usize * num_channels],
            num_channels,
        }
    }

    pub fn size(&self) -> usize {
        self.buf.len() / self.num_channels
    }

    pub fn frame(&self, pos: usize) -> &[f32] {
        let start = pos * self.num_channels;
        &self.buf[start..start + self.num_channels]
    }

    pub fn frame_mut(&mut self, pos: usize) -> &mut [f32] {
        let start = pos * self.num_channels;
        &mut self.buf[start..start + self.num_channels]
    }

    pub fn channels(&self) -> u16 {
        self.num_channels as u16
    }

    pub fn zero(&mut self, pos: usize, len: usize) {
        let start = pos * self.num_channels;
        let end = (pos + len) * self.num_channels;
        self.buf[start..end].fill(0.0);
    }

    pub fn samples(&self) -> &[f32] {
        &self.buf
    }

    pub fn samples_mut(&mut self) -> &mut [f32] {
        &mut self.buf
    }

    pub fn frames_mut(&mut self, pos: usize, len: usize) -> &mut [f32] {
        let start = pos * self.num_channels;
        let end = (pos + len) * self.num_channels;
        &mut self.buf[start..end]
    }
}

pub trait PcmReader {
    fn read(&mut self, data: &mut PcmBuffer, size: u32) -> Result<bool, PcmEngineError>;
}

pub trait PcmWriter {
    fn write(&mut self, data: &PcmBuffer, size: u32) -> Result<(), PcmEngineError>;
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessMeta {
    pub channels: u16,
}

pub trait Processor {
    fn process_frame(&mut self, data: &mut [f32], meta: &ProcessMeta) -> ProcessResult;
}

pub struct PcmEngine {
    buffer: PcmBuffer,
    writer: Option<Box<dyn PcmWriter>>,
    reader: Option<Box<dyn PcmReader>>,
    processed: u64,
    to_drain: u64,
}

impl PcmEngine {
    pub fn new(
        buf_size: u16,
        num_channels: usize,
        reader: Option<Box<dyn PcmReader>>,
        writer: Option<Box<dyn PcmWriter>>,
    ) -> Self {
        Self {
            buffer: PcmBuffer::new(buf_size, num_channels),
            writer,
            reader,
            processed: 0,
            to_drain: 0,
        }
    }

    pub fn apply_process(
        &mut self,
        step: usize,
        processor: &mut dyn Processor,
    ) -> Result<u64, PcmEngineError> {
        if step > self.buffer.size() {
            return Err(PcmEngineError::BufferTooSmall);
        }

        let mut drain = false;
        if let Some(reader) = &mut self.reader {
            let size_to_read = self.buffer.size() as u32;
            let ok = reader.read(&mut self.buffer, size_to_read)?;
            if !ok {
                if self.to_drain != 0 {
                    drain = true;
                } else {
                    return Err(PcmEngineError::NoDataToRead);
                }
            }
        }

        let mut last_pos = 0_usize;
        let meta = ProcessMeta {
            channels: self.buffer.channels(),
        };

        for i in (0..=self.buffer.size() - step).step_by(step) {
            let res = processor.process_frame(self.buffer.frames_mut(i, step), &meta);
            if res == ProcessResult::Processed {
                last_pos += step;
                if drain && self.to_drain != 0 {
                    self.to_drain -= 1;
                    break;
                }
            } else {
                assert!(!drain);
                self.to_drain += 1;
            }
        }

        if let Some(writer) = &mut self.writer {
            writer.write(&self.buffer, last_pos as u32)?;
        }

        self.processed += last_pos as u64;
        Ok(self.processed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VecReader {
        chunks: Vec<Vec<f32>>,
        channels: usize,
    }

    impl PcmReader for VecReader {
        fn read(&mut self, data: &mut PcmBuffer, size: u32) -> Result<bool, PcmEngineError> {
            if data.channels() as usize != self.channels {
                return Err(PcmEngineError::WrongReadBuffer);
            }
            let Some(chunk) = self.chunks.first().cloned() else {
                return Ok(false);
            };
            self.chunks.remove(0);
            let frames = chunk.len() / self.channels;
            data.samples_mut()[..chunk.len()].copy_from_slice(&chunk);
            if frames < size as usize {
                data.zero(frames, size as usize - frames);
            }
            Ok(frames != 0)
        }
    }

    #[derive(Default)]
    struct VecWriter {
        data: Vec<f32>,
    }

    impl PcmWriter for VecWriter {
        fn write(&mut self, data: &PcmBuffer, size: u32) -> Result<(), PcmEngineError> {
            self.data
                .extend_from_slice(&data.samples()[..size as usize * data.channels() as usize]);
            Ok(())
        }
    }

    struct LookaheadOnce {
        calls: usize,
    }

    impl Processor for LookaheadOnce {
        fn process_frame(&mut self, data: &mut [f32], _meta: &ProcessMeta) -> ProcessResult {
            self.calls += 1;
            data[0] += 1.0;
            if self.calls == 1 {
                ProcessResult::LookAhead
            } else {
                ProcessResult::Processed
            }
        }
    }

    #[test]
    fn apply_process_handles_lookahead_and_drain() {
        let reader = VecReader {
            chunks: vec![vec![1.0, 2.0, 3.0, 4.0]],
            channels: 1,
        };
        let writer = VecWriter::default();
        let mut engine = PcmEngine::new(4, 1, Some(Box::new(reader)), Some(Box::new(writer)));
        let mut proc = LookaheadOnce { calls: 0 };

        assert_eq!(3, engine.apply_process(1, &mut proc).unwrap());
        assert_eq!(4, engine.apply_process(1, &mut proc).unwrap());
        assert_eq!(
            Err(PcmEngineError::NoDataToRead),
            engine.apply_process(1, &mut proc)
        );
    }
}
