use crate::{
    AtracdencError,
    at1::{
        bitalloc::Atrac1BitAllocator,
        data::{
            BlockSizeMod, EncodeSettings, NUM_QMF, NUM_SAMPLES, SCALE_TABLE, SPECS_PER_BLOCK,
            SPECS_START_LONG, SPECS_START_SHORT, WindowMode,
        },
        dequantiser::Atrac1Dequantiser,
        mdct::Atrac1Mdct,
        qmf::{Atrac1AnalysisFilterBank, Atrac1SynthesisFilterBank},
    },
    atrac::{
        psy::{create_loudness_curve, track_loudness, track_loudness_mono},
        scale::{ScaledBlock, Scaler},
    },
    bitstream::BitStream,
    container::{CompressedInput, CompressedOutput},
    dsp::transient::TransientDetector,
    pcm::engine::{ProcessMeta, ProcessResult, Processor},
    util::inverted_spectr,
};

const LOUD_FACTOR: f32 = 0.006;

pub struct Atrac1Encoder {
    output: Box<dyn CompressedOutput>,
    settings: EncodeSettings,
    mdct: Atrac1Mdct,
    pcm_low: Vec<Vec<f32>>,
    pcm_mid: Vec<Vec<f32>>,
    pcm_hi: Vec<Vec<f32>>,
    analysis_filter_bank: Vec<Atrac1AnalysisFilterBank>,
    transient_detectors: Vec<[TransientDetector; NUM_QMF]>,
    loudness_curve: Vec<f32>,
    bit_allocs: Vec<Atrac1BitAllocator>,
    scaler: Scaler,
    loudness: f32,
}

impl Atrac1Encoder {
    pub fn new(output: Box<dyn CompressedOutput>, settings: EncodeSettings) -> Self {
        let channels = output.channels().max(1);
        Self {
            output,
            settings,
            mdct: Atrac1Mdct::new(),
            pcm_low: vec![vec![0.0; 256 + 16]; channels],
            pcm_mid: vec![vec![0.0; 256 + 16]; channels],
            pcm_hi: vec![vec![0.0; 512 + 16]; channels],
            analysis_filter_bank: (0..channels)
                .map(|_| Atrac1AnalysisFilterBank::new())
                .collect(),
            transient_detectors: (0..channels)
                .map(|_| {
                    [
                        TransientDetector::new(16, 128),
                        TransientDetector::new(16, 128),
                        TransientDetector::new(16, 256),
                    ]
                })
                .collect(),
            loudness_curve: create_loudness_curve(NUM_SAMPLES),
            bit_allocs: (0..channels)
                .map(|_| Atrac1BitAllocator::new(settings.bfu_idx_const))
                .collect(),
            scaler: Scaler::new(&SCALE_TABLE[..]),
            loudness: LOUD_FACTOR,
        }
    }

    pub fn channels(&self) -> usize {
        self.output.channels().max(1)
    }
}

impl Processor for Atrac1Encoder {
    fn process_frame(
        &mut self,
        data: &mut [f32],
        _meta: &ProcessMeta,
    ) -> Result<ProcessResult, AtracdencError> {
        let channels = self.channels();
        assert!(data.len() >= NUM_SAMPLES * channels);
        let mut window_masks = vec![0_u32; channels];
        let mut block_sizes = vec![BlockSizeMod::default(); channels];
        let mut channel_specs = vec![vec![0.0; NUM_SAMPLES]; channels];
        let mut channel_loudness = vec![0.0; channels];

        for channel in 0..channels {
            let mut src = vec![0.0; NUM_SAMPLES];
            for i in 0..NUM_SAMPLES {
                src[i] = data[i * channels + channel];
            }

            self.analysis_filter_bank[channel].analysis(
                &src,
                &mut self.pcm_low[channel][..128],
                &mut self.pcm_mid[channel][..128],
                &mut self.pcm_hi[channel][..256],
            );

            let mut window_mask = 0_u32;
            if self.settings.window_mode == WindowMode::Auto {
                window_mask |= u32::from(
                    self.transient_detectors[channel][0].detect(&self.pcm_low[channel][..128]),
                );
                let inv_mid = inverted_spectr(&self.pcm_mid[channel][..128]);
                window_mask |=
                    u32::from(self.transient_detectors[channel][1].detect(&inv_mid)) << 1;
                let inv_hi = inverted_spectr(&self.pcm_hi[channel][..256]);
                window_mask |= u32::from(self.transient_detectors[channel][2].detect(&inv_hi)) << 2;
            } else {
                window_mask = self.settings.window_mask;
            }

            window_masks[channel] = window_mask;
            let block_size = BlockSizeMod::new(
                window_mask & 0x1 != 0,
                window_mask & 0x2 != 0,
                window_mask & 0x4 != 0,
            );
            block_sizes[channel] = block_size;

            self.mdct.mdct(
                &mut channel_specs[channel],
                &mut self.pcm_low[channel],
                &mut self.pcm_mid[channel],
                &mut self.pcm_hi[channel],
                &block_size,
            );

            channel_loudness[channel] = channel_specs[channel]
                .iter()
                .zip(&self.loudness_curve)
                .map(|(spec, curve)| spec * spec * curve)
                .sum();
        }

        if channels == 2 && window_masks[0] == 0 && window_masks[1] == 0 {
            self.loudness = track_loudness(self.loudness, channel_loudness[0], channel_loudness[1]);
        } else if window_masks[0] == 0 {
            self.loudness = track_loudness_mono(self.loudness, channel_loudness[0]);
        }

        for channel in 0..channels {
            let scaled_blocks =
                scale_at1_frame(&self.scaler, &channel_specs[channel], &block_sizes[channel]);
            let frame = self.bit_allocs[channel].encode_frame(
                &scaled_blocks,
                &block_sizes[channel],
                self.loudness / LOUD_FACTOR,
            );
            self.output.write_frame(&frame)?;
        }

        Ok(ProcessResult::Processed)
    }
}

pub struct Atrac1Decoder {
    input: Box<dyn CompressedInput>,
    mdct: Atrac1Mdct,
    pcm_low: Vec<Vec<f32>>,
    pcm_mid: Vec<Vec<f32>>,
    pcm_hi: Vec<Vec<f32>>,
    synthesis_filter_bank: Vec<Atrac1SynthesisFilterBank>,
    specs_buf: Vec<Vec<f32>>,
    sum_buf: Vec<Vec<f32>>,
    pcm_value_max: f32,
    pcm_value_min: f32,
}

impl Atrac1Decoder {
    pub fn new(input: Box<dyn CompressedInput>) -> Self {
        let channels = input.channels().max(1);
        Self {
            input,
            mdct: Atrac1Mdct::new(),
            pcm_low: vec![vec![0.0; 256 + 16]; channels],
            pcm_mid: vec![vec![0.0; 256 + 16]; channels],
            pcm_hi: vec![vec![0.0; 512 + 16]; channels],
            synthesis_filter_bank: (0..channels)
                .map(|_| Atrac1SynthesisFilterBank::new())
                .collect(),
            specs_buf: vec![vec![0.0; NUM_SAMPLES]; channels],
            sum_buf: vec![vec![0.0; NUM_SAMPLES]; channels],
            pcm_value_max: 1.0,
            pcm_value_min: -1.0,
        }
    }

    pub fn channels(&self) -> usize {
        self.input.channels().max(1)
    }
}

impl Processor for Atrac1Decoder {
    fn process_frame(
        &mut self,
        data: &mut [f32],
        _meta: &ProcessMeta,
    ) -> Result<ProcessResult, AtracdencError> {
        let channels = self.channels();
        assert!(data.len() >= NUM_SAMPLES * channels);

        for channel in 0..channels {
            let frame = match self.input.read_frame()? {
                Some(frame) => frame,
                None => {
                    // The original atracdenc decode loop processes whole engine
                    // blocks, which can round the requested output length up past
                    // the last physical AEA frame. Emit silence for the missing
                    // frame instead of panicking so the trailing block completes.
                    for i in 0..NUM_SAMPLES {
                        data[i * channels + channel] = 0.0;
                    }
                    continue;
                }
            };
            let mut bitstream = BitStream::from_bytes(&frame);
            let mode = BlockSizeMod::parse(&mut bitstream);
            let specs = &mut self.specs_buf[channel];
            specs.fill(0.0);
            Atrac1Dequantiser::new().dequant(&mut bitstream, &mode, specs);
            self.mdct.imdct(
                specs,
                &mode,
                &mut self.pcm_low[channel],
                &mut self.pcm_mid[channel],
                &mut self.pcm_hi[channel],
            );

            let sum = &mut self.sum_buf[channel];
            self.synthesis_filter_bank[channel].synthesis(
                sum,
                &self.pcm_low[channel][..128],
                &self.pcm_mid[channel][..128],
                &self.pcm_hi[channel][..256],
            );

            for i in 0..NUM_SAMPLES {
                let sample = sum[i].clamp(self.pcm_value_min, self.pcm_value_max);
                data[i * channels + channel] = sample;
            }
        }

        Ok(ProcessResult::Processed)
    }
}

pub fn scale_at1_frame(
    scaler: &Scaler,
    specs: &[f32],
    block_size: &BlockSizeMod,
) -> Vec<ScaledBlock> {
    let mut scaled_blocks = Vec::with_capacity(SPECS_PER_BLOCK.len());
    for band_num in 0..NUM_QMF {
        for block_num in crate::at1::data::BLOCKS_PER_BAND[band_num] as usize
            ..crate::at1::data::BLOCKS_PER_BAND[band_num + 1] as usize
        {
            let spec_num_start = if block_size.short_win(band_num) {
                SPECS_START_SHORT[block_num]
            } else {
                SPECS_START_LONG[block_num]
            } as usize;
            let len = SPECS_PER_BLOCK[block_num] as usize;
            scaled_blocks.push(scaler.scale(&specs[spec_num_start..spec_num_start + len]));
        }
    }
    scaled_blocks
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;
    use crate::container::aea::{AeaInput, AeaOutput};
    use crate::{
        container::ContainerError,
        pcm::engine::{PcmBuffer, PcmEngine, PcmReader, PcmWriter},
    };

    #[derive(Clone, Default)]
    struct SharedOutput {
        frames: Rc<RefCell<Vec<Vec<u8>>>>,
        channels: usize,
    }

    impl CompressedOutput for SharedOutput {
        fn write_frame(&mut self, data: &[u8]) -> Result<(), ContainerError> {
            self.frames.borrow_mut().push(data.to_vec());
            Ok(())
        }

        fn name(&self) -> &str {
            ""
        }

        fn channels(&self) -> usize {
            self.channels
        }
    }

    struct MemoryInput {
        frames: Vec<Vec<u8>>,
        pos: usize,
        channels: usize,
    }

    impl CompressedInput for MemoryInput {
        fn read_frame(&mut self) -> Result<Option<Vec<u8>>, ContainerError> {
            let frame = self.frames.get(self.pos).cloned();
            self.pos += usize::from(frame.is_some());
            Ok(frame)
        }

        fn frame_size(&self) -> usize {
            crate::at1::data::SOUND_UNIT_SIZE as usize
        }

        fn length_in_samples(&self) -> u64 {
            self.frames.len() as u64 * NUM_SAMPLES as u64
        }

        fn name(&self) -> &str {
            ""
        }

        fn channels(&self) -> usize {
            self.channels
        }
    }

    struct BlockReader {
        frames: Vec<Vec<f32>>,
        pos: usize,
        channels: usize,
    }

    impl PcmReader for BlockReader {
        fn read(&mut self, data: &mut PcmBuffer, size: u32) -> Result<bool, AtracdencError> {
            assert_eq!(NUM_SAMPLES, size as usize);
            assert_eq!(self.channels, data.channels() as usize);
            let Some(frame) = self.frames.get(self.pos) else {
                return Ok(false);
            };
            self.pos += 1;
            data.samples_mut()[..frame.len()].copy_from_slice(frame);
            Ok(true)
        }
    }

    #[derive(Default)]
    struct CollectWriter {
        frames: Rc<RefCell<Vec<Vec<f32>>>>,
    }

    impl PcmWriter for CollectWriter {
        fn write(&mut self, data: &PcmBuffer, size: u32) -> Result<(), AtracdencError> {
            self.frames
                .borrow_mut()
                .push(data.samples()[..size as usize * data.channels() as usize].to_vec());
            Ok(())
        }
    }

    #[test]
    fn encoder_writes_one_atrac1_frame_per_channel() {
        let shared = SharedOutput {
            frames: Rc::new(RefCell::new(Vec::new())),
            channels: 1,
        };
        let frames = shared.frames.clone();
        let mut encoder = Atrac1Encoder::new(Box::new(shared), EncodeSettings::default());
        let mut pcm = vec![0.0; NUM_SAMPLES];
        for (i, sample) in pcm.iter_mut().enumerate() {
            *sample = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44_100.0).sin() * 0.1;
        }

        assert_eq!(
            ProcessResult::Processed,
            encoder
                .process_frame(&mut pcm, &ProcessMeta { channels: 1 })
                .unwrap()
        );
        let frames = frames.borrow();
        assert_eq!(1, frames.len());
        assert!(frames[0].len() <= crate::at1::data::SOUND_UNIT_SIZE as usize);
    }

    #[test]
    fn scale_at1_frame_returns_all_bfus() {
        let scaler = Scaler::new(&SCALE_TABLE[..]);
        let specs = vec![0.01; NUM_SAMPLES];
        let blocks = scale_at1_frame(&scaler, &specs, &BlockSizeMod::default());
        assert_eq!(crate::at1::data::MAX_BFUS, blocks.len());
    }

    #[test]
    fn encoded_frame_runs_through_decoder_path() {
        let shared = SharedOutput {
            frames: Rc::new(RefCell::new(Vec::new())),
            channels: 1,
        };
        let frames = shared.frames.clone();
        let mut encoder = Atrac1Encoder::new(Box::new(shared), EncodeSettings::default());
        let mut pcm = vec![0.0; NUM_SAMPLES];
        for (i, sample) in pcm.iter_mut().enumerate() {
            *sample = (2.0 * std::f32::consts::PI * 880.0 * i as f32 / 44_100.0).sin() * 0.05;
        }
        encoder
            .process_frame(&mut pcm, &ProcessMeta { channels: 1 })
            .unwrap();

        let input = MemoryInput {
            frames: frames.borrow().clone(),
            pos: 0,
            channels: 1,
        };
        let mut decoder = Atrac1Decoder::new(Box::new(input));
        let mut decoded = vec![0.0; NUM_SAMPLES];
        assert_eq!(
            ProcessResult::Processed,
            decoder
                .process_frame(&mut decoded, &ProcessMeta { channels: 1 })
                .unwrap()
        );
        assert!(decoded.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn pcm_engine_atrac1_encode_decode_smoke() {
        let channels = 1;
        let input_frames = (0..4)
            .map(|frame| {
                (0..NUM_SAMPLES)
                    .map(|i| {
                        let n = frame * NUM_SAMPLES + i;
                        (2.0 * std::f32::consts::PI * 440.0 * n as f32 / 44_100.0).sin() * 0.05
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let shared = SharedOutput {
            frames: Rc::new(RefCell::new(Vec::new())),
            channels,
        };
        let encoded_frames = shared.frames.clone();
        let reader = BlockReader {
            frames: input_frames,
            pos: 0,
            channels,
        };
        let mut encode_engine = PcmEngine::new(NUM_SAMPLES, channels, Some(Box::new(reader)), None);
        let mut encoder = Atrac1Encoder::new(Box::new(shared), EncodeSettings::default());

        while encode_engine
            .apply_process(NUM_SAMPLES, &mut encoder)
            .is_ok()
        {}
        assert!(encoded_frames.borrow().len() >= 4);

        let decoded_frames = Rc::new(RefCell::new(Vec::new()));
        let writer = CollectWriter {
            frames: decoded_frames.clone(),
        };
        let input = MemoryInput {
            frames: encoded_frames.borrow().clone(),
            pos: 0,
            channels,
        };
        let mut decode_engine = PcmEngine::new(NUM_SAMPLES, channels, None, Some(Box::new(writer)));
        let mut decoder = Atrac1Decoder::new(Box::new(input));

        for _ in 0..encoded_frames.borrow().len() {
            decode_engine
                .apply_process(NUM_SAMPLES, &mut decoder)
                .unwrap();
        }

        let decoded_frames = decoded_frames.borrow();
        assert_eq!(encoded_frames.borrow().len(), decoded_frames.len());
        assert!(
            decoded_frames
                .iter()
                .flatten()
                .all(|sample| sample.is_finite() && *sample >= -1.0 && *sample <= 1.0)
        );
    }

    #[test]
    fn atrac1_file_aea_roundtrip_smoke() {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "atracdenc-at1-roundtrip-{}.aea",
            std::process::id()
        ));

        {
            let file = std::fs::File::create(&path).unwrap();
            let output = AeaOutput::new(file, "rust-at1", 1, 0).unwrap();
            let mut encoder = Atrac1Encoder::new(Box::new(output), EncodeSettings::default());

            for frame in 0..4 {
                let mut pcm = vec![0.0; NUM_SAMPLES];
                for (i, sample) in pcm.iter_mut().enumerate() {
                    let n = frame * NUM_SAMPLES + i;
                    *sample =
                        (2.0 * std::f32::consts::PI * 330.0 * n as f32 / 44_100.0).sin() * 0.04;
                }
                encoder
                    .process_frame(&mut pcm, &ProcessMeta { channels: 1 })
                    .unwrap();
            }
        }

        {
            let file = std::fs::File::open(&path).unwrap();
            let mut input = AeaInput::new(file).unwrap();
            assert_eq!(1, input.channels());
            assert_eq!("rust-at1", input.name());
            assert!(input.read_frame().unwrap().is_some());
        }

        let _ = std::fs::remove_file(path);
    }
}
