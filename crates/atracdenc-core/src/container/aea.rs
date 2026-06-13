use std::io::{self, Read, Seek, SeekFrom, Write};

use super::{CompressedInput, CompressedOutput, ContainerError};

pub const AEA_META_SIZE: usize = 2048;
pub const AEA_FRAME_SIZE: usize = 212;
pub const AEA_PREROLL_FRAMES: u64 = 5;

fn aea_title(header: &[u8; AEA_META_SIZE]) -> &str {
    let title = &header[4..20];
    let len = title.iter().position(|b| *b == 0).unwrap_or(title.len());
    std::str::from_utf8(&title[..len]).unwrap_or("")
}

pub struct AeaOutput<W: Write + Seek> {
    inner: Option<W>,
    header: [u8; AEA_META_SIZE],
    first_write: bool,
    written_frames: u32,
}

impl<W: Write + Seek> AeaOutput<W> {
    pub fn new(
        mut inner: W,
        title: &str,
        num_channels: usize,
        num_frames: u32,
    ) -> Result<Self, ContainerError> {
        let mut header = [0_u8; AEA_META_SIZE];
        header[0..4].copy_from_slice(&[0x00, 0x08, 0x00, 0x00]);
        let title_bytes = title.as_bytes();
        let title_len = title_bytes.len().min(15);
        header[4..4 + title_len].copy_from_slice(&title_bytes[..title_len]);
        header[19] = 0;
        header[260..264].copy_from_slice(&num_frames.to_ne_bytes());
        header[264] = num_channels as u8;

        inner.write_all(&header)?;
        inner.write_all(&[0_u8; AEA_FRAME_SIZE])?;

        Ok(Self {
            inner: Some(inner),
            header,
            first_write: true,
            written_frames: 0,
        })
    }

    pub fn finalize(&mut self) -> Result<(), ContainerError> {
        self.header[260..264].copy_from_slice(&self.written_frames.to_ne_bytes());
        let Some(inner) = &mut self.inner else {
            return Ok(());
        };
        let pos = inner.stream_position()?;
        inner.seek(SeekFrom::Start(260))?;
        inner.write_all(&self.header[260..264])?;
        inner.seek(SeekFrom::Start(pos))?;
        Ok(())
    }

    pub fn into_inner(mut self) -> Result<W, ContainerError> {
        self.finalize()?;
        self.inner.take().ok_or(ContainerError::AlreadyConsumed)
    }
}

impl<W: Write + Seek> Drop for AeaOutput<W> {
    fn drop(&mut self) {
        let _ = self.finalize();
    }
}

impl<W: Write + Seek> CompressedOutput for AeaOutput<W> {
    fn write_frame(&mut self, data: &[u8]) -> Result<(), ContainerError> {
        if self.first_write {
            self.first_write = false;
            return Ok(());
        }

        let mut frame = data.to_vec();
        frame.resize(AEA_FRAME_SIZE, 0);
        self.inner
            .as_mut()
            .ok_or(ContainerError::AlreadyConsumed)?
            .write_all(&frame)?;
        self.written_frames += 1;
        Ok(())
    }

    fn name(&self) -> &str {
        aea_title(&self.header)
    }

    fn channels(&self) -> usize {
        self.header[264] as usize
    }
}

pub struct AeaInput<R: Read + Seek> {
    inner: R,
    header: [u8; AEA_META_SIZE],
    data_len: u64,
}

impl<R: Read + Seek> AeaInput<R> {
    pub fn new(mut inner: R) -> Result<Self, ContainerError> {
        let mut header = [0_u8; AEA_META_SIZE];
        inner.read_exact(&mut header)?;
        if header[0..4] != [0x00, 0x08, 0x00, 0x00] || header[264] >= 3 {
            return Err(ContainerError::InvalidInput("invalid AEA header"));
        }
        let end = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(AEA_META_SIZE as u64))?;
        Ok(Self {
            inner,
            header,
            data_len: end.saturating_sub(AEA_META_SIZE as u64),
        })
    }
}

impl<R: Read + Seek> CompressedInput for AeaInput<R> {
    fn read_frame(&mut self) -> Result<Option<Vec<u8>>, ContainerError> {
        let mut frame = vec![0_u8; AEA_FRAME_SIZE];
        match self.inner.read_exact(&mut frame) {
            Ok(()) => Ok(Some(frame)),
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
            Err(err) => Err(ContainerError::Io(err)),
        }
    }

    fn frame_size(&self) -> usize {
        AEA_FRAME_SIZE
    }

    fn length_in_samples(&self) -> u64 {
        let channels = if self.header[264] == 0 {
            1
        } else {
            u64::from(self.header[264])
        };
        512 * ((self.data_len / AEA_FRAME_SIZE as u64 / channels)
            .saturating_sub(AEA_PREROLL_FRAMES))
    }

    fn name(&self) -> &str {
        aea_title(&self.header)
    }

    fn channels(&self) -> usize {
        self.header[264] as usize
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn aea_header_dummy_skip_and_readback() {
        let mut out =
            AeaOutput::new(Cursor::new(Vec::new()), "test-title-that-is-long", 2, 0).unwrap();
        out.write_frame(&[0xaa; AEA_FRAME_SIZE]).unwrap();
        out.write_frame(&[0xbb; AEA_FRAME_SIZE]).unwrap();
        out.write_frame(&[0xcc; AEA_FRAME_SIZE]).unwrap();
        assert_eq!("test-title-that", out.name());
        let bytes = out.into_inner().unwrap().into_inner();

        assert_eq!(&[0x00, 0x08, 0x00, 0x00], &bytes[0..4]);
        assert_eq!(2_u32.to_ne_bytes(), bytes[260..264]);
        assert_eq!(2, bytes[264]);
        assert_eq!(AEA_META_SIZE + AEA_FRAME_SIZE * 3, bytes.len());

        let mut input = AeaInput::new(Cursor::new(bytes)).unwrap();
        assert_eq!(2, input.channels());
        assert_eq!("test-title-that", input.name());
        assert_eq!(
            Some(vec![0_u8; AEA_FRAME_SIZE]),
            input.read_frame().unwrap()
        );
        assert_eq!(
            Some(vec![0xbb; AEA_FRAME_SIZE]),
            input.read_frame().unwrap()
        );
        assert_eq!(
            Some(vec![0xcc; AEA_FRAME_SIZE]),
            input.read_frame().unwrap()
        );
        assert_eq!(None, input.read_frame().unwrap());
    }
}
