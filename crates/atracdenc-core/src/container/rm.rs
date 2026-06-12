use std::io::{self, Seek, SeekFrom, Write};

use super::CompressedOutput;

const RMF_HEADER_SZ: usize = 18;
const PROP_HEADER_SZ: usize = 50;
const CODEC_DATA_SZ: usize = 92;
const RA_MIME: &[u8] = b"audio/x-pn-realaudio\0";
const RA_DESC: &[u8] = b"Audio Stream\0";
const MDPR_HEADER_SZ: usize = 42 + RA_MIME.len() + RA_DESC.len() + CODEC_DATA_SZ;
const DATA_HEADER_SZ: usize = 18;

pub struct RmOutput<W: Write + Seek> {
    inner: Option<W>,
    frame_duration: f64,
    timestamp: f64,
    frame_num: u32,
    data_header_pos: u64,
    frame_size: u32,
}

impl<W: Write + Seek> RmOutput<W> {
    pub fn new(
        mut inner: W,
        num_channels: usize,
        num_frames: u32,
        frame_size: u32,
        joint_stereo: bool,
    ) -> io::Result<Self> {
        let frame_duration = 1000.0 * 1024.0 / 44_100.0;
        let bitrate = (8.0 * f64::from(frame_size) * 44_100.0 / 1024.0) as u32;
        write_rmf(&mut inner)?;
        write_prop(&mut inner, frame_size, num_frames, bitrate, frame_duration)?;
        write_mdpr(
            &mut inner,
            frame_size,
            num_frames,
            num_channels,
            joint_stereo,
            bitrate,
            frame_duration,
        )?;
        let data_header_pos = inner.stream_position()?;
        write_data(&mut inner, num_frames)?;
        Ok(Self {
            inner: Some(inner),
            frame_duration,
            timestamp: 0.0,
            frame_num: 0,
            data_header_pos,
            frame_size,
        })
    }

    pub fn finalize(&mut self) -> io::Result<()> {
        let Some(inner) = &mut self.inner else {
            return Ok(());
        };
        let end = inner.stream_position()?;
        let data_chunk_size = end.saturating_sub(self.data_header_pos);
        if data_chunk_size <= u64::from(u32::MAX) {
            let pos = inner.stream_position()?;
            inner.seek(SeekFrom::Start(self.data_header_pos + 4))?;
            inner.write_all(&(data_chunk_size as u32).to_be_bytes())?;
            inner.seek(SeekFrom::Start(pos))?;
        }
        Ok(())
    }

    pub fn into_inner(mut self) -> io::Result<W> {
        self.finalize()?;
        Ok(self.inner.take().expect("RM output inner already taken"))
    }

    fn write_audio_packet(&mut self, data: &[u8]) -> io::Result<()> {
        let inner = self.inner.as_mut().expect("RM output inner already taken");
        match self.frame_num % 3 {
            0 => {
                push_u16_to(inner, 0)?;
                push_u16_to(inner, (3 * data.len() + 12) as u16)?;
                push_u16_to(inner, 0)?;
                push_u32_to(inner, self.timestamp as u32)?;
                inner.write_all(&[0, 0x02])?;
            }
            2 => {
                self.timestamp += self.frame_duration * 3.0;
            }
            _ => {}
        }
        inner.write_all(data)
    }
}

impl<W: Write + Seek> CompressedOutput for RmOutput<W> {
    fn write_frame(&mut self, data: &[u8]) -> io::Result<()> {
        if data.len() != self.frame_size as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unexpected RM frame size",
            ));
        }
        let scrambled = scramble_data(data);
        self.write_audio_packet(&scrambled)?;
        self.frame_num += 1;
        Ok(())
    }

    fn name(&self) -> &str {
        ""
    }

    fn channels(&self) -> usize {
        0
    }
}

impl<W: Write + Seek> Drop for RmOutput<W> {
    fn drop(&mut self) {
        let _ = self.finalize();
    }
}

fn write_rmf<W: Write>(w: &mut W) -> io::Result<()> {
    w.write_all(b".RMF")?;
    push_u32_to(w, RMF_HEADER_SZ as u32)?;
    push_u16_to(w, 0)?;
    push_u32_to(w, 0)?;
    push_u32_to(w, 4)
}

fn write_prop<W: Write>(
    w: &mut W,
    frame_size: u32,
    num_frames: u32,
    bitrate: u32,
    frame_duration: f64,
) -> io::Result<()> {
    w.write_all(b"PROP")?;
    push_u32_to(w, PROP_HEADER_SZ as u32)?;
    push_u16_to(w, 0)?;
    push_u32_to(w, bitrate)?;
    push_u32_to(w, bitrate)?;
    push_u32_to(w, frame_size)?;
    push_u32_to(w, frame_size)?;
    push_u32_to(w, num_frames)?;
    push_u32_to(w, (f64::from(num_frames) * frame_duration) as u32)?;
    push_u32_to(w, 0)?;
    push_u32_to(w, 0)?;
    push_u32_to(w, (RMF_HEADER_SZ + PROP_HEADER_SZ + MDPR_HEADER_SZ) as u32)?;
    push_u16_to(w, 1)?;
    push_u16_to(w, 1 | 2)
}

fn write_mdpr<W: Write>(
    w: &mut W,
    frame_size: u32,
    num_frames: u32,
    num_channels: usize,
    joint_stereo: bool,
    bitrate: u32,
    frame_duration: f64,
) -> io::Result<()> {
    w.write_all(b"MDPR")?;
    push_u32_to(w, MDPR_HEADER_SZ as u32)?;
    push_u16_to(w, 0)?;
    push_u16_to(w, 0)?;
    push_u32_to(w, bitrate)?;
    push_u32_to(w, bitrate)?;
    push_u32_to(w, frame_size)?;
    push_u32_to(w, frame_size)?;
    push_u32_to(w, 0)?;
    push_u32_to(w, 0)?;
    push_u32_to(w, (f64::from(num_frames) * frame_duration) as u32)?;
    w.write_all(&[RA_DESC.len() as u8])?;
    w.write_all(RA_DESC)?;
    w.write_all(&[RA_MIME.len() as u8])?;
    w.write_all(RA_MIME)?;
    let codec = codec_data(frame_size, num_channels, joint_stereo, bitrate);
    w.write_all(&codec)
}

fn write_data<W: Write>(w: &mut W, num_frames: u32) -> io::Result<()> {
    debug_assert_eq!(DATA_HEADER_SZ, 18);
    w.write_all(b"DATA")?;
    push_u32_to(w, u32::MAX)?;
    push_u16_to(w, 0)?;
    push_u32_to(w, num_frames)?;
    push_u32_to(w, 0)
}

fn codec_data(
    frame_size: u32,
    num_channels: usize,
    joint_stereo: bool,
    bitrate: u32,
) -> [u8; CODEC_DATA_SZ] {
    let mut buf = [0_u8; CODEC_DATA_SZ];
    buf[0..4].copy_from_slice(&((CODEC_DATA_SZ - 4) as u32).to_be_bytes());
    buf[4..8].copy_from_slice(&[b'.', b'r', b'a', 0xfd]);
    buf[8..10].copy_from_slice(&5_u16.to_be_bytes());
    buf[12..16].copy_from_slice(b".ra5");
    buf[16..20].copy_from_slice(&0x01b5_3530_u32.to_be_bytes());
    buf[20..22].copy_from_slice(&5_u16.to_be_bytes());
    buf[26..28].copy_from_slice(&2_u16.to_be_bytes());
    buf[28..32].copy_from_slice(&(frame_size * 3).to_be_bytes());
    buf[32..36].copy_from_slice(&0x0005_1540_u32.to_be_bytes());
    buf[36..40].copy_from_slice(&(bitrate / 8 * 60).to_be_bytes());
    buf[40..44].copy_from_slice(&(bitrate / 8 * 60).to_be_bytes());
    buf[44..46].copy_from_slice(&1_u16.to_be_bytes());
    buf[46..48].copy_from_slice(&(frame_size as u16 * 3).to_be_bytes());
    buf[48..50].copy_from_slice(&(frame_size as u16).to_be_bytes());
    buf[54..56].copy_from_slice(&44_100_u16.to_be_bytes());
    buf[58..60].copy_from_slice(&44_100_u16.to_be_bytes());
    buf[62..64].copy_from_slice(&16_u16.to_be_bytes());
    buf[64..66].copy_from_slice(&2_u16.to_be_bytes());
    buf[66..74].copy_from_slice(b"genratrc");
    buf[74] = 0x01;
    buf[75] = 0x07;
    buf[78..82].copy_from_slice(&10_u32.to_be_bytes());
    buf[82..86].copy_from_slice(&4_u32.to_be_bytes());
    buf[86..88].copy_from_slice(&(1024_u16 * num_channels as u16).to_be_bytes());
    buf[88..90].copy_from_slice(&0x088e_u16.to_be_bytes());
    buf[90..92].copy_from_slice(&(if joint_stereo { 0x12_u16 } else { 0x02_u16 }).to_be_bytes());
    buf
}

fn scramble_data(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for chunk in input.chunks(4) {
        let mut bytes = [0_u8; 4];
        bytes[..chunk.len()].copy_from_slice(chunk);
        let v = u32::from_le_bytes(bytes) ^ 0x0361_7f53;
        out.extend_from_slice(&v.to_le_bytes()[..chunk.len()]);
    }
    out
}

fn push_u16_to<W: Write>(w: &mut W, x: u16) -> io::Result<()> {
    w.write_all(&x.to_be_bytes())
}

fn push_u32_to<W: Write>(w: &mut W, x: u32) -> io::Result<()> {
    w.write_all(&x.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn rm_header_packet_and_data_backfill() {
        let mut out = RmOutput::new(Cursor::new(Vec::new()), 2, 3, 16, true).unwrap();
        out.write_frame(&[0; 16]).unwrap();
        out.write_frame(&[1; 16]).unwrap();
        out.write_frame(&[2; 16]).unwrap();
        let bytes = out.into_inner().unwrap().into_inner();

        assert_eq!(b".RMF", &bytes[0..4]);
        assert_eq!((RMF_HEADER_SZ as u32).to_be_bytes(), bytes[4..8]);
        assert_eq!(b"PROP", &bytes[18..22]);
        assert_eq!(b"MDPR", &bytes[68..72]);
        let data_pos = RMF_HEADER_SZ + PROP_HEADER_SZ + MDPR_HEADER_SZ;
        assert_eq!(b"DATA", &bytes[data_pos..data_pos + 4]);
        assert_eq!(
            (DATA_HEADER_SZ as u32 + 12 + 16 * 3).to_be_bytes(),
            bytes[data_pos + 4..data_pos + 8]
        );
        assert_eq!(3_u32.to_be_bytes(), bytes[data_pos + 10..data_pos + 14]);
        assert_eq!(
            0_u16.to_be_bytes(),
            bytes[data_pos + DATA_HEADER_SZ..data_pos + DATA_HEADER_SZ + 2]
        );
        assert_eq!(
            (3 * 16_u16 + 12).to_be_bytes(),
            bytes[data_pos + DATA_HEADER_SZ + 2..data_pos + DATA_HEADER_SZ + 4]
        );
    }
}
