use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::Path,
};

use crate::{
    AtracdencError,
    pcm::engine::{PcmBuffer, PcmEngineError, PcmReader, PcmWriter},
    util::to_int,
};

pub struct WavReader {
    inner: hound::WavReader<BufReader<File>>,
    spec: hound::WavSpec,
}

impl WavReader {
    pub fn open(path: &Path) -> Result<Self, hound::Error> {
        let inner = hound::WavReader::open(path)?;
        let spec = inner.spec();
        Ok(Self { inner, spec })
    }

    pub fn channels(&self) -> u16 {
        self.spec.channels
    }

    pub fn sample_rate(&self) -> u32 {
        self.spec.sample_rate
    }

    pub fn bits_per_sample(&self) -> u16 {
        self.spec.bits_per_sample
    }

    pub fn total_samples(&self) -> u64 {
        u64::from(self.inner.duration()) / u64::from(self.spec.channels)
    }
}

impl PcmReader for WavReader {
    fn read(&mut self, data: &mut PcmBuffer, size: u32) -> Result<bool, AtracdencError> {
        if data.channels() != self.spec.channels {
            return Err(PcmEngineError::ChannelMismatch.into());
        }

        let total = size as usize * self.spec.channels as usize;
        let mut read = 0;
        for dst in &mut data.samples_mut()[..total] {
            match self.inner.samples::<i16>().next() {
                Some(Ok(sample)) => {
                    *dst = f32::from(sample) / 32768.0;
                    read += 1;
                }
                Some(Err(e)) => {
                    return Err(PcmEngineError::Io(std::io::Error::other(e.to_string())).into());
                }
                None => break,
            }
        }

        if read == 0 {
            return Ok(false);
        }

        if read < total {
            data.samples_mut()[read..total].fill(0.0);
        }
        Ok(true)
    }
}

pub struct WavWriter {
    inner: hound::WavWriter<BufWriter<File>>,
    channels: u16,
}

impl WavWriter {
    pub fn create(path: &Path, channels: u16, sample_rate: u32) -> Result<Self, hound::Error> {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let inner = hound::WavWriter::create(path, spec)?;
        Ok(Self { inner, channels })
    }

    pub fn finalize(self) -> Result<(), hound::Error> {
        self.inner.finalize()
    }
}

impl PcmWriter for WavWriter {
    fn write(&mut self, data: &PcmBuffer, size: u32) -> Result<(), AtracdencError> {
        if data.channels() != self.channels {
            return Err(PcmEngineError::ChannelMismatch.into());
        }

        let total = size as usize * data.channels() as usize;
        if total == 0 {
            return Ok(());
        }

        let scale_max = 32767.0 / 32768.0;
        let scale_factor = 32768.0;

        let mut writer = self.inner.get_i16_writer(total as u32);
        for sample in &data.samples()[..total] {
            let clipped = sample.clamp(-1.0, scale_max);
            let encoded =
                to_int(clipped * scale_factor).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            writer.write_sample(encoded);
        }
        writer.flush().map_err(|e| {
            AtracdencError::from(PcmEngineError::Io(std::io::Error::other(e.to_string())))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("atracdenc-{name}-{}.wav", std::process::id()));
        path
    }

    #[test]
    fn wav_round_trip_normalizes_i16_samples() {
        let path = temp_path("roundtrip");
        {
            let mut writer = WavWriter::create(&path, 1, 44_100).unwrap();
            let mut buf = PcmBuffer::new(3, 1);
            buf.samples_mut()
                .copy_from_slice(&[-1.0, 0.0, 32767.0 / 32768.0]);
            writer.write(&buf, 3).unwrap();
            writer.finalize().unwrap();
        }
        {
            let mut reader = WavReader::open(&path).unwrap();
            assert_eq!(1, reader.channels());
            assert_eq!(44_100, reader.sample_rate());
            assert_eq!(16, reader.bits_per_sample());
            assert_eq!(3, reader.total_samples());
            let mut buf = PcmBuffer::new(3, 1);
            assert!(reader.read(&mut buf, 3).unwrap());
            assert!((buf.samples()[0] + 1.0).abs() < 1.0e-6);
            assert_eq!(0.0, buf.samples()[1]);
            assert!((buf.samples()[2] - 32767.0 / 32768.0).abs() < 1.0e-6);
        }
        let _ = std::fs::remove_file(path);
    }
}
