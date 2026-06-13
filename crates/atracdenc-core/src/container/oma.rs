use std::io::Write;

use super::{CompressedOutput, ContainerError};

pub const OMA_HEADER_SIZE: usize = 96;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OmaCodec {
    Atrac3,
    Atrac3Plus,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OmaChannelFormat {
    Mono = 0,
    Stereo = 1,
    StereoJoint = 2,
    Channels3 = 3,
    Channels4 = 4,
    Channels6 = 5,
    Channels7 = 6,
    Channels8 = 7,
}

fn sample_rate_idx(sample_rate: u32) -> Result<u32, ContainerError> {
    match sample_rate {
        32_000 => Ok(0),
        44_100 => Ok(1),
        48_000 => Ok(2),
        88_200 => Ok(3),
        96_000 => Ok(4),
        _ => Err(ContainerError::UnsupportedSampleRate),
    }
}

fn channel_idx(format: OmaChannelFormat) -> u32 {
    match format {
        OmaChannelFormat::Mono => 0,
        OmaChannelFormat::Stereo => 1,
        OmaChannelFormat::Channels3 => 2,
        OmaChannelFormat::Channels4 => 3,
        OmaChannelFormat::Channels6 => 4,
        OmaChannelFormat::Channels7 => 5,
        OmaChannelFormat::Channels8 => 6,
        OmaChannelFormat::StereoJoint => 1,
    }
}

fn params(
    codec: OmaCodec,
    sample_rate: u32,
    channel_format: OmaChannelFormat,
    frame_size: u32,
) -> Result<u32, ContainerError> {
    let sr_idx = sample_rate_idx(sample_rate)?;
    match codec {
        OmaCodec::Atrac3 => {
            if !matches!(
                channel_format,
                OmaChannelFormat::Stereo | OmaChannelFormat::StereoJoint
            ) {
                return Err(ContainerError::InvalidInput(
                    "ATRAC3 OMA requires stereo channel format",
                ));
            }
            let js = u32::from(channel_format == OmaChannelFormat::StereoJoint);
            let frame_units = frame_size / 8;
            if frame_units > 0x3ff {
                return Err(ContainerError::InvalidInput("OMA frame too large"));
            }
            Ok((js << 17) | (sr_idx << 13) | frame_units)
        }
        OmaCodec::Atrac3Plus => {
            let frame_units = (frame_size - 8) / 8;
            if frame_units > 0x3ff {
                return Err(ContainerError::InvalidInput("OMA frame too large"));
            }
            Ok(
                (1 << 24)
                    | (sr_idx << 13)
                    | ((channel_idx(channel_format) + 1) << 10)
                    | frame_units,
            )
        }
    }
}

pub struct OmaOutput<W: Write> {
    inner: W,
    codec: OmaCodec,
    channel_format: OmaChannelFormat,
    frame_size: usize,
}

impl<W: Write> OmaOutput<W> {
    pub fn new(
        mut inner: W,
        codec: OmaCodec,
        sample_rate: u32,
        channel_format: OmaChannelFormat,
        frame_size: u32,
    ) -> Result<Self, ContainerError> {
        let mut header = [0_u8; OMA_HEADER_SIZE];
        header[0..3].copy_from_slice(b"EA3");
        header[3] = 1;
        header[5] = OMA_HEADER_SIZE as u8;
        header[6] = 0xff;
        header[7] = 0xff;
        header[32..36].copy_from_slice(
            &params(codec, sample_rate, channel_format, frame_size)?.to_be_bytes(),
        );
        inner.write_all(&header)?;
        Ok(Self {
            inner,
            codec,
            channel_format,
            frame_size: frame_size as usize,
        })
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> CompressedOutput for OmaOutput<W> {
    fn write_frame(&mut self, data: &[u8]) -> Result<(), ContainerError> {
        if data.len() < self.frame_size {
            return Err(ContainerError::InvalidInput("short OMA frame"));
        }
        self.inner.write_all(&data[..self.frame_size])?;
        Ok(())
    }

    fn name(&self) -> &str {
        match self.codec {
            OmaCodec::Atrac3 => "ATRAC3",
            OmaCodec::Atrac3Plus => "ATRAC3PLUS",
        }
    }

    fn channels(&self) -> usize {
        match self.channel_format {
            OmaChannelFormat::Mono => 1,
            OmaChannelFormat::Stereo | OmaChannelFormat::StereoJoint => 2,
            OmaChannelFormat::Channels3 => 3,
            OmaChannelFormat::Channels4 => 4,
            OmaChannelFormat::Channels6 => 6,
            OmaChannelFormat::Channels7 => 7,
            OmaChannelFormat::Channels8 => 8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oma_atrac3_header_and_frame() {
        let mut out = OmaOutput::new(
            Vec::new(),
            OmaCodec::Atrac3,
            44_100,
            OmaChannelFormat::StereoJoint,
            192,
        )
        .unwrap();
        out.write_frame(&[0x55; 192]).unwrap();
        let bytes = out.into_inner();
        assert_eq!(b"EA3", &bytes[0..3]);
        assert_eq!(1, bytes[3]);
        assert_eq!(96, bytes[5]);
        assert_eq!([0xff, 0xff], bytes[6..8]);
        let expected = (1_u32 << 17) | (1_u32 << 13) | (192_u32 / 8);
        assert_eq!(expected.to_be_bytes(), bytes[32..36]);
        assert_eq!(96 + 192, bytes.len());
    }

    #[test]
    fn oma_atrac3plus_header() {
        let out = OmaOutput::new(
            Vec::new(),
            OmaCodec::Atrac3Plus,
            44_100,
            OmaChannelFormat::Stereo,
            376,
        )
        .unwrap();
        let bytes = out.into_inner();
        let expected = (1_u32 << 24) | (1_u32 << 13) | (2_u32 << 10) | ((376_u32 - 8) / 8);
        assert_eq!(expected.to_be_bytes(), bytes[32..36]);
    }
}
