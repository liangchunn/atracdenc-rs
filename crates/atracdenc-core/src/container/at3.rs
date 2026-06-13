use std::io::{self, Seek, SeekFrom, Write};

use super::CompressedOutput;

const WAVE_SAMPLE_RATE: u32 = 44_100;
const AT3_SAMPLES_PER_FRAME: u32 = 1024;
const AT3P_SAMPLES_PER_FRAME: u32 = 2048;
const AT3_HEADER_SIZE: usize = 76;
const AT3P_HEADER_SIZE: usize = 80;

const ATRAC3PLUS_SUBFORMAT_GUID: [u8; 16] = [
    0xBF, 0xAA, 0x23, 0xE9, 0x58, 0xCB, 0x71, 0x44, 0xA1, 0x19, 0xFF, 0xFA, 0x01, 0xE4, 0xCE, 0x62,
];

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum At3Kind {
    Atrac3 { joint_stereo: bool },
    Atrac3Plus,
}

pub struct At3Output<W: Write + Seek> {
    inner: Option<W>,
    kind: At3Kind,
    frame_size: u32,
    frames_written: u64,
    channels: usize,
}

impl<W: Write + Seek> At3Output<W> {
    pub fn atrac3(
        mut inner: W,
        channels: usize,
        num_frames: u32,
        frame_size: u32,
        joint_stereo: bool,
    ) -> io::Result<Self> {
        let mut header = Vec::with_capacity(AT3_HEADER_SIZE);
        let file_size = AT3_HEADER_SIZE as u64 + u64::from(num_frames) * u64::from(frame_size);
        if file_size >= u64::from(u32::MAX) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "RIFF file too large",
            ));
        }
        write_riff_prefix(&mut header, (file_size - 8) as u32, 32);
        push_u16(&mut header, 0x0270);
        push_u16(&mut header, channels as u16);
        push_u32(&mut header, WAVE_SAMPLE_RATE);
        push_u32(
            &mut header,
            frame_size * WAVE_SAMPLE_RATE / AT3_SAMPLES_PER_FRAME,
        );
        push_u16(&mut header, frame_size as u16);
        push_u16(&mut header, 0);
        push_u16(&mut header, 14);
        push_u16(&mut header, 1);
        push_u32(&mut header, 0x1000);
        let mode = u16::from(joint_stereo);
        push_u16(&mut header, mode);
        push_u16(&mut header, mode);
        push_u16(&mut header, 1);
        push_u16(&mut header, 0);
        header.extend_from_slice(b"fact");
        push_u32(&mut header, 8);
        push_u32(&mut header, num_frames * AT3_SAMPLES_PER_FRAME);
        push_u32(&mut header, AT3_SAMPLES_PER_FRAME);
        header.extend_from_slice(b"data");
        push_u32(&mut header, num_frames * frame_size);
        debug_assert_eq!(AT3_HEADER_SIZE, header.len());
        inner.write_all(&header)?;
        Ok(Self {
            inner: Some(inner),
            kind: At3Kind::Atrac3 { joint_stereo },
            frame_size,
            frames_written: 0,
            channels,
        })
    }

    pub fn atrac3plus(
        mut inner: W,
        channels: usize,
        num_frames: u32,
        frame_size: u32,
    ) -> io::Result<Self> {
        let channels_u16 = u16::try_from(channels).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "too many ATRAC3plus channels")
        })?;
        let frame_size_u16 = u16::try_from(frame_size).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "ATRAC3plus frame size too large",
            )
        })?;
        let mut header = Vec::with_capacity(AT3P_HEADER_SIZE);
        let file_size = AT3P_HEADER_SIZE as u64 + u64::from(num_frames) * u64::from(frame_size);
        if file_size >= u64::from(u32::MAX) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "RIFF file too large",
            ));
        }
        write_riff_prefix(&mut header, (file_size - 8) as u32, 40);
        push_u16(&mut header, 0xfffe);
        push_u16(&mut header, channels_u16);
        push_u32(&mut header, WAVE_SAMPLE_RATE);
        push_u32(
            &mut header,
            frame_size * WAVE_SAMPLE_RATE / AT3P_SAMPLES_PER_FRAME,
        );
        push_u16(&mut header, frame_size_u16);
        push_u16(&mut header, 16);
        push_u16(&mut header, 22);
        push_u16(&mut header, 16);
        push_u32(&mut header, wave_channel_mask(channels));
        header.extend_from_slice(&ATRAC3PLUS_SUBFORMAT_GUID);
        header.extend_from_slice(b"fact");
        push_u32(&mut header, 4);
        push_u32(&mut header, num_frames * AT3P_SAMPLES_PER_FRAME);
        header.extend_from_slice(b"data");
        push_u32(&mut header, num_frames * frame_size);
        debug_assert_eq!(AT3P_HEADER_SIZE, header.len());
        inner.write_all(&header)?;
        Ok(Self {
            inner: Some(inner),
            kind: At3Kind::Atrac3Plus,
            frame_size,
            frames_written: 0,
            channels,
        })
    }

    pub fn finalize(&mut self) -> io::Result<()> {
        let (header_size, samples_per_frame, total_offset, data_offset) = match self.kind {
            At3Kind::Atrac3 { .. } => (AT3_HEADER_SIZE, AT3_SAMPLES_PER_FRAME, 60_u64, 72_u64),
            At3Kind::Atrac3Plus => (AT3P_HEADER_SIZE, AT3P_SAMPLES_PER_FRAME, 68_u64, 76_u64),
        };
        let actual_file_size =
            header_size as u64 + self.frames_written * u64::from(self.frame_size);
        if actual_file_size >= u64::from(u32::MAX) {
            return Ok(());
        }
        let Some(inner) = &mut self.inner else {
            return Ok(());
        };
        patch_u32(inner, 4, (actual_file_size - 8) as u32)?;
        patch_u32(
            inner,
            total_offset,
            self.frames_written as u32 * samples_per_frame,
        )?;
        patch_u32(
            inner,
            data_offset,
            self.frames_written as u32 * self.frame_size,
        )?;
        inner.seek(SeekFrom::End(0))?;
        Ok(())
    }

    pub fn into_inner(mut self) -> io::Result<W> {
        self.finalize()?;
        Ok(self.inner.take().expect("AT3 output inner already taken"))
    }
}

impl<W: Write + Seek> CompressedOutput for At3Output<W> {
    fn write_frame(&mut self, data: &[u8]) -> io::Result<()> {
        if data.len() != self.frame_size as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unexpected AT3 frame size",
            ));
        }
        self.inner
            .as_mut()
            .expect("AT3 output inner already taken")
            .write_all(data)?;
        self.frames_written += 1;
        Ok(())
    }

    fn name(&self) -> &str {
        ""
    }

    fn channels(&self) -> usize {
        self.channels
    }
}

impl<W: Write + Seek> Drop for At3Output<W> {
    fn drop(&mut self) {
        let _ = self.finalize();
    }
}

fn write_riff_prefix(buf: &mut Vec<u8>, chunk_size: u32, fmt_size: u32) {
    buf.extend_from_slice(b"RIFF");
    push_u32(buf, chunk_size);
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    push_u32(buf, fmt_size);
}

fn wave_channel_mask(channels: usize) -> u32 {
    match channels {
        1 => 0x0000_0004,
        2 => 0x0000_0003,
        _ => 0,
    }
}

fn push_u16(buf: &mut Vec<u8>, x: u16) {
    buf.extend_from_slice(&x.to_le_bytes());
}

fn push_u32(buf: &mut Vec<u8>, x: u32) {
    buf.extend_from_slice(&x.to_le_bytes());
}

fn patch_u32<W: Write + Seek>(inner: &mut W, offset: u64, value: u32) -> io::Result<()> {
    inner.seek(SeekFrom::Start(offset))?;
    inner.write_all(&value.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn atrac3_wave_header_and_backfill() {
        let mut out = At3Output::atrac3(Cursor::new(Vec::new()), 2, 0, 192, true).unwrap();
        out.write_frame(&[0x11; 192]).unwrap();
        out.write_frame(&[0x22; 192]).unwrap();
        let bytes = out.into_inner().unwrap().into_inner();

        assert_eq!(b"RIFF", &bytes[0..4]);
        assert_eq!(b"WAVE", &bytes[8..12]);
        assert_eq!(0x0270_u16.to_le_bytes(), bytes[20..22]);
        assert_eq!(192_u16.to_le_bytes(), bytes[32..34]);
        assert_eq!(14_u16.to_le_bytes(), bytes[36..38]);
        assert_eq!(b"fact", &bytes[52..56]);
        assert_eq!((2 * 1024_u32).to_le_bytes(), bytes[60..64]);
        assert_eq!((2 * 192_u32).to_le_bytes(), bytes[72..76]);
        assert_eq!(AT3_HEADER_SIZE + 384, bytes.len());
    }

    #[test]
    fn atrac3plus_wave_header() {
        let out = At3Output::atrac3plus(Cursor::new(Vec::new()), 2, 0, 376).unwrap();
        let bytes = out.into_inner().unwrap().into_inner();
        assert_eq!(b"RIFF", &bytes[0..4]);
        assert_eq!(0xfffe_u16.to_le_bytes(), bytes[20..22]);
        assert_eq!(22_u16.to_le_bytes(), bytes[36..38]);
        assert_eq!(0x0000_0003_u32.to_le_bytes(), bytes[40..44]);
        assert_eq!(ATRAC3PLUS_SUBFORMAT_GUID, bytes[44..60]);
        assert_eq!(b"fact", &bytes[60..64]);
        assert_eq!(b"data", &bytes[72..76]);
    }

    #[test]
    fn atrac3plus_rejects_oversized_u16_fields() {
        assert_eq!(
            "too many ATRAC3plus channels",
            At3Output::atrac3plus(Cursor::new(Vec::new()), usize::from(u16::MAX) + 1, 0, 376)
                .err()
                .unwrap()
                .to_string()
        );
        assert_eq!(
            "ATRAC3plus frame size too large",
            At3Output::atrac3plus(Cursor::new(Vec::new()), 2, 0, u32::from(u16::MAX) + 1)
                .err()
                .unwrap()
                .to_string()
        );
    }
}
