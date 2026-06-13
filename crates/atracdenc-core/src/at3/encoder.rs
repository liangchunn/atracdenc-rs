use std::io::Write;

use crate::{
    at3::{
        bitstream::{Atrac3BitStreamWriter, SingleChannelElement, TonalBlock},
        data::{
            self, BLOCKS_PER_BAND, EncodeSettings, GAIN_LEVEL, NUM_QMF, NUM_SAMPLES, SCALE_TABLE,
            SPECS_PER_BLOCK, SPECS_START_LONG, TonalVal,
        },
        mdct::{Atrac3Mdct, relation_to_idx},
        qmf::Atrac3AnalysisFilterBank,
        yaml_log::write_float_seq,
    },
    atrac::{
        psy::{
            ENERGY_FLOOR, calc_spectral_flatness_per_bfu, create_loudness_curve, track_loudness,
            track_loudness_mono,
        },
        scale::{ScaledBlock, Scaler},
    },
    container::CompressedOutput,
    dsp::{
        gain::GainPoint,
        transient::{CurveBuilderCtx, GainCurvePoint, analyze_gain, calc_curve},
        upsampler::SpectralUpsampler,
    },
    pcm::engine::{ProcessMeta, ProcessResult, Processor},
};

pub const LOUD_FACTOR: f32 = 0.006;

pub struct Atrac3Encoder {
    output: Box<dyn CompressedOutput>,
    settings: EncodeSettings,
    bitstream: Atrac3BitStreamWriter,
    analysis_filter_bank: Vec<Atrac3AnalysisFilterBank>,
    mdct: Atrac3Mdct,
    upsampler: SpectralUpsampler,
    scaler: Scaler,
    loudness_curve: Vec<f32>,
    loudness: f32,
    lookahead_pending: bool,
    lookahead_buf: Vec<[[f32; 640]; NUM_QMF]>,
    pcm_bands: Vec<[[f32; 512]; NUM_QMF]>,
    curve_ctx: Vec<[CurveBuilderCtx; NUM_QMF]>,
    prev_overlap_gain_scale: Vec<[f32; NUM_QMF]>,
    single_channel_elements: Vec<SingleChannelElement>,
    yaml_log: Option<Box<dyn Write>>,
    frame_num: u64,
}

impl Atrac3Encoder {
    pub fn new(output: Box<dyn CompressedOutput>, settings: EncodeSettings) -> Self {
        Self::with_yaml_log(output, settings, None)
    }

    pub fn with_yaml_log(
        output: Box<dyn CompressedOutput>,
        settings: EncodeSettings,
        yaml_log: Option<Box<dyn Write>>,
    ) -> Self {
        let max_channels = 2;
        Self {
            output,
            settings,
            bitstream: Atrac3BitStreamWriter::with_alloc_mode(
                settings.container_params,
                settings.bfu_idx_const,
                settings.bfu_alloc_mode,
            ),
            analysis_filter_bank: (0..max_channels)
                .map(|_| Atrac3AnalysisFilterBank::new())
                .collect(),
            mdct: Atrac3Mdct::new(),
            upsampler: SpectralUpsampler::with_default_eps(11_025.0, 800.0),
            scaler: Scaler::new(&SCALE_TABLE[..]),
            loudness_curve: create_loudness_curve(NUM_SAMPLES),
            loudness: LOUD_FACTOR,
            lookahead_pending: true,
            lookahead_buf: vec![[[0.0; 640]; NUM_QMF]; max_channels],
            pcm_bands: vec![[[0.0; 512]; NUM_QMF]; max_channels],
            curve_ctx: vec![[CurveBuilderCtx::default(); NUM_QMF]; max_channels],
            prev_overlap_gain_scale: vec![[1.0; NUM_QMF]; max_channels],
            single_channel_elements: Vec::with_capacity(max_channels),
            yaml_log,
            frame_num: 0,
        }
    }

    pub fn channels(&self) -> usize {
        self.output
            .channels()
            .max(usize::from(self.settings.source_channels.max(1)))
            .clamp(1, 2)
    }

    fn analysis_into_lookahead(
        &mut self,
        channel: usize,
        qmf_offset: usize,
        data: &[f32],
        channels: usize,
    ) {
        let mut src = [0.0_f32; NUM_SAMPLES];
        for i in 0..NUM_SAMPLES {
            src[i] = data[i * channels + channel] * 0.25;
        }

        let [sub0, sub1, sub2, sub3] = &mut self.lookahead_buf[channel];
        let mut subs = [
            &mut sub0[qmf_offset..qmf_offset + 256],
            &mut sub1[qmf_offset..qmf_offset + 256],
            &mut sub2[qmf_offset..qmf_offset + 256],
            &mut sub3[qmf_offset..qmf_offset + 256],
        ];
        self.analysis_filter_bank[channel].analysis(&src, &mut subs);
    }

    fn copy_current_slot_to_mdct_input(&mut self, channels: usize) {
        for channel in 0..channels {
            for band in 0..NUM_QMF {
                self.pcm_bands[channel][band][256..512]
                    .copy_from_slice(&self.lookahead_buf[channel][band][128..384]);
            }
        }
    }

    fn matrix_current_mdct_input(&mut self) {
        let (left_channels, right_channels) = self.pcm_bands.split_at_mut(1);
        let left = &mut left_channels[0];
        let right = &mut right_channels[0];
        for band in 0..NUM_QMF {
            for sample in 256..512 {
                let l = left[band][sample];
                let r = right[band][sample];
                left[band][sample] = (l + r) * 0.5;
                right[band][sample] = (l - r) * 0.5;
            }
        }
    }

    fn build_gain_input(&self, channels: usize) -> Vec<[[f32; 512]; NUM_QMF]> {
        let mut gain_input = vec![[[0.0_f32; 512]; NUM_QMF]; channels];
        for (channel, chan_data) in gain_input.iter_mut().enumerate().take(channels) {
            for (band, band_data) in chan_data.iter_mut().enumerate().take(NUM_QMF) {
                band_data.copy_from_slice(&self.lookahead_buf[channel][band][0..512]);
            }
        }

        if self.settings.container_params.joint_stereo && channels == 2 {
            let (left_channels, right_channels) = gain_input.split_at_mut(1);
            let left = &mut left_channels[0];
            let right = &mut right_channels[0];
            for band in 0..NUM_QMF {
                for sample in 0..512 {
                    let l = left[band][sample];
                    let r = right[band][sample];
                    left[band][sample] = (l + r) * 0.5;
                    right[band][sample] = (l - r) * 0.5;
                }
            }
        }

        gain_input
    }

    fn create_subband_info(
        &mut self,
        channel: usize,
        up_input: &[[f32; 512]; NUM_QMF],
    ) -> Vec<Vec<GainPoint>> {
        const MIN_SCORE: f32 = 1.9;
        const MIN_SIGNAL_THRESHOLD: f32 = 1.0e-4;
        const MIN_HFR_FOR_AMPLIFY: f32 = 0.3;

        let mut subband_info = vec![Vec::new(); NUM_QMF];

        if let Some(log) = self.yaml_log.as_deref_mut() {
            let _ = writeln!(log, "  - channel: {channel}");
            let _ = writeln!(log, "    bands:");
        }

        for band in 0..NUM_QMF {
            if let Some(log) = self.yaml_log.as_deref_mut() {
                let _ = writeln!(log, "      - band: {band}");
            }

            let result = self.upsampler.process(&up_input[band]);
            if result.high_freq_ratio < SpectralUpsampler::HIGH_FREQ_THRESHOLD {
                if let Some(log) = self.yaml_log.as_deref_mut() {
                    let _ = writeln!(
                        log,
                        "        skip: low_hfr  # high_freq_ratio {:.4} < threshold",
                        result.high_freq_ratio
                    );
                }
                self.curve_ctx[channel][band].last_level = 0.0;
                continue;
            }

            let mut gain_low = Vec::new();
            let mut gain_high = Vec::new();
            let gain = analyze_gain(
                &result.signal[1024..3072],
                32,
                true,
                Some(&mut gain_low),
                Some(&mut gain_high),
            );
            let next_level = analyze_gain(&result.signal[3072..3136], 1, true, None, None)
                .first()
                .copied();

            let cur_hpf_energy = gain.iter().copied().sum::<f32>() / gain.len() as f32;
            let prev_hpf_energy = self.curve_ctx[channel][band].last_hpf_energy;
            self.curve_ctx[channel][band].last_hpf_energy = cur_hpf_energy;
            let hpf_overlap_ratio = if cur_hpf_energy > 1.0e-9 && prev_hpf_energy > 1.0e-9 {
                prev_hpf_energy / cur_hpf_energy
            } else {
                1.0
            };
            let dynamic_min_score = MIN_SCORE * hpf_overlap_ratio.clamp(1.0, 1.5);

            let prev = &self.pcm_bands[channel][band][..256];
            let cur = &self.pcm_bands[channel][band][256..512];
            let overlap_e = prev.iter().map(|x| x * x).sum::<f32>();
            let cur_e = cur.iter().map(|x| x * x).sum::<f32>();
            let overlap_ratio = overlap_e / (cur_e + 1.0e-9);

            if let Some(log) = self.yaml_log.as_deref_mut() {
                let _ = writeln!(
                    log,
                    "        pcm_qmf:  # 256 raw QMF samples, non-modulated, non-windowed"
                );
                let _ = write!(log, "          ");
                let _ = write_float_seq(log, cur, 6);
                let _ = writeln!(log);
                let _ = writeln!(
                    log,
                    "        high_freq_ratio: {:.4}",
                    result.high_freq_ratio
                );
                let _ = writeln!(
                    log,
                    "        overlap_ratio: {:.4}  # prev_E/cur_E full-band; >1 means prev frame louder",
                    overlap_ratio
                );
                let _ = writeln!(
                    log,
                    "        hpf_overlap_ratio: {:.4}  # prev_HPF/cur_HPF; used for transient suppression decisions",
                    hpf_overlap_ratio
                );
                let _ = writeln!(log, "        dynamic_min_score: {:.4}", dynamic_min_score);
                let _ = writeln!(log, "        next_level: {:.4}", next_level.unwrap_or(0.0));
                let _ = write!(log, "        gain: ");
                let _ = write_float_seq(log, &gain, 4);
                let _ = writeln!(log, "  # 32 subframe RMS values");
            }

            let prev_target = self.curve_ctx[channel][band].last_target;
            let mut curve_points = calc_curve(
                &gain,
                &mut self.curve_ctx[channel][band],
                next_level,
                dynamic_min_score,
                self.yaml_log.as_deref_mut(),
                Some(&gain_low),
                Some(&gain_high),
            );
            let cur_target = self.curve_ctx[channel][band].last_target;

            if curve_points.is_empty() {
                if let Some(log) = self.yaml_log.as_deref_mut() {
                    let _ = writeln!(log, "        skip: no_curve");
                }
                continue;
            }

            if let Some(log) = self.yaml_log.as_deref_mut() {
                let _ = writeln!(log, "        curve_raw:");
                for point in &curve_points {
                    let _ = writeln!(
                        log,
                        "          - {{level: {}, loc: {}}}",
                        point.level, point.location
                    );
                }
            }

            let max_gain = gain.iter().copied().fold(0.0_f32, f32::max);
            if max_gain < MIN_SIGNAL_THRESHOLD {
                if let Some(log) = self.yaml_log.as_deref_mut() {
                    let _ = writeln!(
                        log,
                        "        skip: below_min_signal  # maxGain {:.6}",
                        max_gain
                    );
                }
                curve_points.clear();
            }

            if result.high_freq_ratio < MIN_HFR_FOR_AMPLIFY {
                if let Some(log) = self.yaml_log.as_deref_mut() {
                    let _ = writeln!(log, "        skip: amplify_low_hfr");
                }
                curve_points.clear();
            }

            if let Some(log) = self.yaml_log.as_deref_mut() {
                let _ = writeln!(log, "        max_gain: {:.4}", max_gain);
            }

            if band >= 3 {
                if let Some(log) = self.yaml_log.as_deref_mut() {
                    let _ = writeln!(
                        log,
                        "        skip: band_ge_3  # inaudible HF; gain modulation disabled"
                    );
                }
                curve_points.clear();
            }

            if band < 3 {
                add_point_zero_guarded(&gain, prev_target, cur_target, &mut curve_points);
            }

            if curve_points.len() >= 2
                && curve_points[0].location == 0
                && curve_points[0].level == curve_points[1].level
            {
                curve_points.remove(0);
            }

            if curve_points.len() > data::SubbandInfo::MAX_GAIN_POINTS_NUM {
                curve_points.truncate(data::SubbandInfo::MAX_GAIN_POINTS_NUM);
            }

            if let Some(log) = self.yaml_log.as_deref_mut() {
                let _ = writeln!(log, "        curve_final:");
                for point in &curve_points {
                    let _ = writeln!(
                        log,
                        "          - {{level: {}, loc: {}}}",
                        point.level, point.location
                    );
                }
            }

            subband_info[band] = curve_points
                .into_iter()
                .map(|p| GainPoint {
                    level: p.level,
                    location: p.location,
                })
                .collect();
        }

        subband_info
    }

    fn shift_lookahead(&mut self, channels: usize) {
        for channel in 0..channels {
            for band in 0..NUM_QMF {
                self.lookahead_buf[channel][band].copy_within(256..640, 0);
            }
        }
    }
}

impl Processor for Atrac3Encoder {
    fn process_frame(&mut self, data: &mut [f32], meta: &ProcessMeta) -> ProcessResult {
        let channels = usize::from(meta.channels).clamp(1, self.channels());
        assert!(channels <= 2);
        assert!(data.len() >= NUM_SAMPLES * channels);

        let qmf_offset = if self.lookahead_pending { 128 } else { 384 };
        for channel in 0..channels {
            self.analysis_into_lookahead(channel, qmf_offset, data, channels);
        }

        if self.lookahead_pending {
            self.lookahead_pending = false;
            return ProcessResult::LookAhead;
        }

        if let Some(log) = self.yaml_log.as_deref_mut() {
            let time_sec = self.frame_num as f32 * NUM_SAMPLES as f32 / 44_100.0;
            let _ = writeln!(log, "---");
            let _ = writeln!(log, "frame: {}", self.frame_num);
            let _ = writeln!(log, "time: {time_sec:.3}  # seconds");
            let _ = writeln!(log, "channels:");
        }

        let gain_input = self.build_gain_input(channels);
        self.copy_current_slot_to_mdct_input(channels);
        let joint_stereo = self.settings.container_params.joint_stereo && channels == 2;
        if joint_stereo {
            self.matrix_current_mdct_input();
        }

        self.single_channel_elements.clear();
        let mut channel_loudness = vec![0.0_f32; channels];
        for (channel, channel_loudness) in channel_loudness.iter_mut().enumerate().take(channels) {
            let mut specs = vec![0.0_f32; NUM_SAMPLES];
            let mut sce = SingleChannelElement::new(NUM_QMF);
            if !self.settings.no_gain_control {
                sce.subband_info = self.create_subband_info(channel, &gain_input[channel]);
            }

            for band in 0..NUM_QMF {
                let gain_energy = Atrac3Mdct::calc_gain_energy_scale(
                    &self.pcm_bands[channel][band][..256],
                    &self.pcm_bands[channel][band][256..512],
                    &sce.subband_info[band],
                    self.prev_overlap_gain_scale[channel][band],
                );
                sce.gain_energy_scale[band] = gain_energy.scale;
                self.prev_overlap_gain_scale[channel][band] = gain_energy.next_overlap_scale;
            }

            if !self.settings.no_gain_control
                && let Some(log) = self.yaml_log.as_deref_mut()
            {
                let _ = writeln!(log, "    gain_energy_scale:");
                for band in 0..NUM_QMF {
                    let scale = sce.gain_energy_scale[band];
                    let next_overlap = self.prev_overlap_gain_scale[channel][band];
                    let _ = writeln!(
                        log,
                        "      - {{band: {band}, prev_half: {:.6}, cur_half: {:.6}, frame: {:.6}, next_overlap: {:.6}}}",
                        scale.prev_half, scale.cur_half, scale.frame, next_overlap
                    );
                }
            }

            {
                let [band0, band1, band2, band3] = &mut self.pcm_bands[channel];
                let mut bands = [
                    &mut band0[..],
                    &mut band1[..],
                    &mut band2[..],
                    &mut band3[..],
                ];
                let gain_points = [
                    &sce.subband_info[0][..],
                    &sce.subband_info[1][..],
                    &sce.subband_info[2][..],
                    &sce.subband_info[3][..],
                ];
                self.mdct
                    .mdct_with_gain(&mut specs, &mut bands, &gain_points);
            }

            let mdct_energy = specs.iter().map(|spec| spec * spec).collect::<Vec<_>>();
            for (i, energy) in mdct_energy.iter().copied().enumerate() {
                let band = i / 256;
                *channel_loudness +=
                    energy * sce.gain_energy_scale[band].frame * self.loudness_curve[i];
            }
            sce.loudness = *channel_loudness;

            if !self.settings.no_tonal_components {
                let flatness_per_bfu = calc_spectral_flatness_per_bfu(
                    &mdct_energy,
                    &SPECS_START_LONG,
                    &SPECS_PER_BLOCK,
                    data::MAX_BFUS,
                    ENERGY_FLOOR,
                );
                let tonal_components = extract_tonal_components(&mut specs, &flatness_per_bfu);
                map_tonal_components(&self.scaler, &tonal_components, &mut sce.tonal_blocks);
            }

            sce.scaled_blocks = scale_at3_frame(&self.scaler, &specs);
            self.single_channel_elements.push(sce);
        }

        if channels == 2 && !self.settings.container_params.joint_stereo {
            self.loudness = track_loudness(self.loudness, channel_loudness[0], channel_loudness[1]);
        } else {
            self.loudness = track_loudness_mono(self.loudness, channel_loudness[0]);
        }

        if self.settings.container_params.joint_stereo && channels == 1 {
            self.single_channel_elements
                .push(SingleChannelElement::new(1));
        }

        self.bitstream
            .write_sound_unit(
                self.output.as_mut(),
                &self.single_channel_elements,
                self.loudness / LOUD_FACTOR,
            )
            .expect("failed to write ATRAC3 frame");

        self.shift_lookahead(channels);
        self.frame_num += 1;
        ProcessResult::Processed
    }
}

fn build_subframe_divisors(points: &[GainCurvePoint], out: &mut [f32; 32]) {
    let mut sample_div = [1.0_f32; 256];
    let mut pos = 0_usize;

    for (i, point) in points.iter().enumerate() {
        let last_pos = (point.location << data::LOC_SCALE) as usize;
        let mut level = data::GAIN_LEVEL[point.level as usize];
        let next_level = points
            .get(i + 1)
            .map(|p| p.level as i32)
            .unwrap_or(data::EXPONENT_OFFSET);
        let inc_pos = next_level - point.level as i32 + data::GAIN_INTERPOLATION_POS_SHIFT;
        let gain_inc = data::GAIN_INTERPOLATION[inc_pos as usize];

        while pos < last_pos && pos < sample_div.len() {
            sample_div[pos] = level;
            pos += 1;
        }
        while pos < last_pos + data::LOC_SZ as usize && pos < sample_div.len() {
            sample_div[pos] = level;
            level *= gain_inc;
            pos += 1;
        }
    }

    for sf in 0..32 {
        out[sf] = sample_div[sf * 8..sf * 8 + 8].iter().sum::<f32>() / 8.0;
    }
}

fn calc_curve_early_mismatch_score(gain: &[f32], target: f32, points: &[GainCurvePoint]) -> f32 {
    if gain.len() != 32 || target <= 1.0e-9 {
        return 0.0;
    }

    let mut div = [1.0_f32; 32];
    build_subframe_divisors(points, &mut div);

    let max_loc = points.iter().map(|p| p.location).max().unwrap_or(0);
    let eval_sf = 32_u32.min(3_u32.max(max_loc + 3)) as usize;
    const EPS: f32 = 1.0e-9;

    let mut fit = 0.0;
    for sf in 0..eval_sf {
        let modulated = gain[sf] / div[sf].max(EPS);
        let e = (modulated.max(EPS) / target.max(EPS)).log2();
        fit += e * e;
    }
    fit /= eval_sf as f32;

    let mut leak = 0.0;
    let mut weight_sum = 0.0;
    for sf in 0..eval_sf.saturating_sub(1) {
        let a = div[sf].max(EPS).log2();
        let b = div[sf + 1].max(EPS).log2();
        let d = b - a;
        let weight = 0.5 * (gain[sf] + gain[sf + 1]);
        leak += d * d * weight;
        weight_sum += weight;
    }
    if weight_sum > EPS {
        leak /= weight_sum;
    }

    fit + 0.25 * leak
}

fn add_point_zero_guarded(
    gain: &[f32],
    prev_target: f32,
    cur_target: f32,
    curve_points: &mut Vec<GainCurvePoint>,
) {
    let curve_before = curve_points.clone();
    let mut hpf_rms_next_mod = 0.0;
    let mut hpf_rms_next_mod_valid = false;

    if let Some(first) = curve_points.first() {
        if first.location > 0 {
            let n_before = first.location as usize;
            let divisor = data::GAIN_LEVEL[first.level as usize];
            hpf_rms_next_mod = gain[..n_before].iter().copied().sum::<f32>() / n_before as f32;
            hpf_rms_next_mod /= divisor;
            hpf_rms_next_mod_valid = true;
        }
    } else if !gain.is_empty() {
        hpf_rms_next_mod = gain.iter().copied().sum::<f32>() / gain.len() as f32;
        hpf_rms_next_mod_valid = true;
    }

    if !(hpf_rms_next_mod_valid && prev_target > 1.0e-6 && hpf_rms_next_mod > 1.0e-6) {
        return;
    }

    let point0_level = u32::from(relation_to_idx(prev_target / hpf_rms_next_mod));
    let mut point0_changed = false;
    if let Some(point) = curve_points.iter_mut().find(|p| p.location == 0) {
        if point.level != point0_level {
            point.level = point0_level;
            point0_changed = true;
        }
    } else if point0_level != data::EXPONENT_OFFSET as u32 || !curve_points.is_empty() {
        curve_points.insert(
            0,
            GainCurvePoint {
                level: point0_level,
                location: 0,
            },
        );
        point0_changed = true;
    }

    if !point0_changed {
        return;
    }

    let score_before = calc_curve_early_mismatch_score(gain, cur_target, &curve_before);
    let score_after = calc_curve_early_mismatch_score(gain, cur_target, curve_points);
    const POINT0_WORSE_TOL: f32 = 0.02;
    const BOUNDARY_KEEP_MARGIN: f32 = 0.20;
    const EPS: f32 = 1.0e-9;

    let first_level = |points: &[GainCurvePoint]| {
        points
            .first()
            .map(|p| p.level)
            .unwrap_or(data::EXPONENT_OFFSET as u32)
    };
    let desired_scale = limit_rel(prev_target / hpf_rms_next_mod);
    let scale_before = data::GAIN_LEVEL[first_level(&curve_before) as usize];
    let scale_after = data::GAIN_LEVEL[first_level(curve_points) as usize];
    let boundary_err_before = (scale_before.max(EPS) / desired_scale.max(EPS))
        .log2()
        .abs();
    let boundary_err_after = (scale_after.max(EPS) / desired_scale.max(EPS)).log2().abs();
    let keep_by_boundary = boundary_err_after + BOUNDARY_KEEP_MARGIN < boundary_err_before;

    if !keep_by_boundary && score_after > score_before * (1.0 + POINT0_WORSE_TOL) {
        *curve_points = curve_before;
    }
}

pub fn limit_rel(x: f32) -> f32 {
    x.clamp(GAIN_LEVEL[15], GAIN_LEVEL[0])
}

pub fn extract_tonal_components(specs: &mut [f32], flatness_per_bfu: &[f32]) -> Vec<TonalVal> {
    const FLATNESS_THRESHOLD: f32 = 0.01;
    const MAX_TONAL_LEN: u32 = 5;
    let mut res = Vec::new();

    for block_num in 8..29_usize {
        if block_num >= flatness_per_bfu.len() {
            break;
        }
        if flatness_per_bfu[block_num] >= FLATNESS_THRESHOLD {
            continue;
        }

        let spec_num_start = SPECS_START_LONG[block_num] as usize;
        let block_len = SPECS_PER_BLOCK[block_num] as usize;
        let spec_num_end = spec_num_start + block_len;
        if spec_num_start >= spec_num_end || spec_num_end > specs.len() {
            continue;
        }

        let max_len = (MAX_TONAL_LEN as usize).min(block_len);
        let mut best_score = -1.0_f32;
        let mut best_start = spec_num_start;
        let mut best_len = 1_usize;

        for start in spec_num_start..spec_num_end {
            let max_len_for_start = max_len.min(spec_num_end - start);
            let mut score = 0.0;
            for len in 1..=max_len_for_start {
                score += specs[start + len - 1].abs();
                if score > best_score {
                    best_score = score;
                    best_start = start;
                    best_len = len;
                }
            }
        }

        if best_score <= 0.0 {
            continue;
        }

        for n in 0..best_len {
            let pos = best_start + n;
            res.push(TonalVal {
                pos: pos as u16,
                val: f64::from(specs[pos]),
                bfu: block_num as u8,
            });
            specs[pos] = 0.0;
        }
    }

    res
}

pub fn map_tonal_components(
    scaler: &Scaler,
    tonal_components: &[TonalVal],
    component_map: &mut Vec<TonalBlock>,
) {
    let mut i = 0;
    while i < tonal_components.len() {
        let start_pos = i;
        let mut cur_pos;
        loop {
            cur_pos = tonal_components[i].pos;
            i += 1;
            if !(i < tonal_components.len()
                && tonal_components[i].pos == cur_pos + 1
                && i - start_pos < 7)
            {
                break;
            }
        }

        let len = i - start_pos;
        let mut tmp = [0.0_f32; 8];
        for j in 0..len {
            tmp[j] = tonal_components[start_pos + j].val as f32;
        }
        let scaled_block = scaler.scale(&tmp[..len]);
        component_map.push(TonalBlock {
            val: tonal_components[start_pos],
            scaled_block,
        });
    }
}

pub fn matrixing(left: &mut [[f32; 256]; 4], right: &mut [[f32; 256]; 4]) {
    for subband in 0..4 {
        for sample in 0..256 {
            let l = left[subband][sample];
            let r = right[subband][sample];
            left[subband][sample] = (l + r) * 0.5;
            right[subband][sample] = (l - r) * 0.5;
        }
    }
}

pub fn scale_at3_frame(scaler: &Scaler, specs: &[f32]) -> Vec<ScaledBlock> {
    let mut scaled_blocks = Vec::with_capacity(SPECS_PER_BLOCK.len());
    for band_num in 0..NUM_QMF {
        for block_num in BLOCKS_PER_BAND[band_num] as usize..BLOCKS_PER_BAND[band_num + 1] as usize
        {
            let spec_num_start = SPECS_START_LONG[block_num] as usize;
            let len = SPECS_PER_BLOCK[block_num] as usize;
            scaled_blocks.push(scaler.scale(&specs[spec_num_start..spec_num_start + len]));
        }
    }
    scaled_blocks
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        io::{self, Write},
        rc::Rc,
    };

    use super::*;
    use crate::at3::{
        bitstream::{Atrac3BitStreamWriter, SingleChannelElement},
        data::{LP2, SCALE_TABLE},
    };
    use crate::container::CompressedOutput;

    #[derive(Clone, Default)]
    struct SharedOutput {
        frames: Rc<RefCell<Vec<Vec<u8>>>>,
        channels: usize,
    }

    #[derive(Clone, Default)]
    struct SharedLog {
        data: Rc<RefCell<Vec<u8>>>,
    }

    impl CompressedOutput for SharedOutput {
        fn write_frame(&mut self, data: &[u8]) -> std::io::Result<()> {
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

    impl Write for SharedLog {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.data.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn limit_rel_clamps_to_atrac3_gain_bounds() {
        assert_eq!(GAIN_LEVEL[0], limit_rel(99.0));
        assert_eq!(GAIN_LEVEL[15], limit_rel(0.0));
        assert_eq!(1.0, limit_rel(1.0));
    }

    #[test]
    fn extract_tonal_components_picks_strongest_low_flatness_run_and_zeros_specs() {
        let mut specs = vec![0.0; 1024];
        let block = 8;
        let start = SPECS_START_LONG[block] as usize;
        specs[start + 2] = 1.0;
        specs[start + 3] = -3.0;
        specs[start + 4] = 2.0;
        let mut flatness = vec![1.0; 32];
        flatness[block] = 0.0;

        let tonal = extract_tonal_components(&mut specs, &flatness);

        assert_eq!(5, tonal.len());
        assert_eq!(start as u16, tonal[0].pos);
        assert_eq!(block as u8, tonal[0].bfu);
        assert!(specs[start..start + 5].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn map_tonal_components_groups_contiguous_positions_up_to_seven_values() {
        let scaler = Scaler::new(&SCALE_TABLE[..]);
        let tonal = (0..8)
            .map(|i| TonalVal {
                pos: 100 + i,
                val: if i % 2 == 0 { 0.5 } else { -0.25 },
                bfu: 10,
            })
            .collect::<Vec<_>>();
        let mut mapped = Vec::new();

        map_tonal_components(&scaler, &tonal, &mut mapped);

        assert_eq!(2, mapped.len());
        assert_eq!(7, mapped[0].scaled_block.values.len());
        assert_eq!(1, mapped[1].scaled_block.values.len());
        assert_eq!(100, mapped[0].val.pos);
        assert_eq!(107, mapped[1].val.pos);
    }

    #[test]
    fn matrixing_converts_lr_to_mid_side() {
        let mut left = [[0.0; 256]; 4];
        let mut right = [[0.0; 256]; 4];
        left[2][10] = 0.75;
        right[2][10] = 0.25;

        matrixing(&mut left, &mut right);

        assert_eq!(0.5, left[2][10]);
        assert_eq!(0.25, right[2][10]);
    }

    #[test]
    fn extracted_tonals_map_into_bitstream_frame() {
        let scaler = Scaler::new(&SCALE_TABLE[..]);
        let mut specs = vec![0.0; 1024];
        let block = 8;
        let start = SPECS_START_LONG[block] as usize;
        specs[start] = 0.5;
        specs[start + 1] = -0.25;
        let mut flatness = vec![1.0; 32];
        flatness[block] = 0.0;

        let tonals = extract_tonal_components(&mut specs, &flatness);
        let mut sce = SingleChannelElement::new(4);
        map_tonal_components(&scaler, &tonals, &mut sce.tonal_blocks);

        let mut writer = Atrac3BitStreamWriter::new(LP2, 0);
        let frame = writer.build_sound_unit_frame(&[sce], 1.0).unwrap();
        assert_eq!(LP2.frame_sz as usize, frame.len());
    }

    #[test]
    fn scale_at3_frame_returns_all_bfus() {
        let scaler = Scaler::new(&SCALE_TABLE[..]);
        let specs = vec![0.01; NUM_SAMPLES];
        let blocks = scale_at3_frame(&scaler, &specs);
        assert_eq!(data::MAX_BFUS, blocks.len());
    }

    #[test]
    fn atrac3_encoder_writes_frame_after_lookahead() {
        let shared = SharedOutput {
            frames: Rc::new(RefCell::new(Vec::new())),
            channels: 1,
        };
        let frames = shared.frames.clone();
        let settings = EncodeSettings {
            source_channels: 1,
            no_gain_control: true,
            no_tonal_components: true,
            ..EncodeSettings::default()
        };
        let mut encoder = Atrac3Encoder::new(Box::new(shared), settings);

        let mut pcm0 = vec![0.0_f32; NUM_SAMPLES];
        let mut pcm1 = vec![0.0_f32; NUM_SAMPLES];
        for i in 0..NUM_SAMPLES {
            pcm0[i] = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44_100.0).sin() * 0.05;
            pcm1[i] = (2.0 * std::f32::consts::PI * 660.0 * i as f32 / 44_100.0).sin() * 0.05;
        }

        assert_eq!(
            ProcessResult::LookAhead,
            encoder.process_frame(&mut pcm0, &ProcessMeta { channels: 1 })
        );
        assert_eq!(0, frames.borrow().len());
        assert_eq!(
            ProcessResult::Processed,
            encoder.process_frame(&mut pcm1, &ProcessMeta { channels: 1 })
        );

        let frames = frames.borrow();
        assert_eq!(1, frames.len());
        assert_eq!(LP2.frame_sz as usize, frames[0].len());
    }

    #[test]
    fn atrac3_encoder_gain_control_path_writes_frame_after_lookahead() {
        let shared = SharedOutput {
            frames: Rc::new(RefCell::new(Vec::new())),
            channels: 1,
        };
        let frames = shared.frames.clone();
        let settings = EncodeSettings {
            source_channels: 1,
            no_tonal_components: true,
            ..EncodeSettings::default()
        };
        let mut encoder = Atrac3Encoder::new(Box::new(shared), settings);

        let mut pcm0 = vec![0.0_f32; NUM_SAMPLES];
        let mut pcm1 = vec![0.0_f32; NUM_SAMPLES];
        for i in 0..NUM_SAMPLES {
            let amp0 = if i < 512 { 0.01 } else { 0.12 };
            let amp1 = if i < 256 { 0.12 } else { 0.02 };
            pcm0[i] = (2.0 * std::f32::consts::PI * 4_400.0 * i as f32 / 44_100.0).sin() * amp0;
            pcm1[i] = (2.0 * std::f32::consts::PI * 5_200.0 * i as f32 / 44_100.0).sin() * amp1;
        }

        assert_eq!(
            ProcessResult::LookAhead,
            encoder.process_frame(&mut pcm0, &ProcessMeta { channels: 1 })
        );
        assert_eq!(
            ProcessResult::Processed,
            encoder.process_frame(&mut pcm1, &ProcessMeta { channels: 1 })
        );

        let frames = frames.borrow();
        assert_eq!(1, frames.len());
        assert_eq!(LP2.frame_sz as usize, frames[0].len());
    }

    #[test]
    fn atrac3_encoder_yaml_log_emits_frame_channel_and_gain_sections() {
        let shared = SharedOutput {
            frames: Rc::new(RefCell::new(Vec::new())),
            channels: 1,
        };
        let log = SharedLog::default();
        let log_data = log.data.clone();
        let settings = EncodeSettings {
            source_channels: 1,
            no_tonal_components: true,
            ..EncodeSettings::default()
        };
        let mut encoder =
            Atrac3Encoder::with_yaml_log(Box::new(shared), settings, Some(Box::new(log)));

        let mut pcm0 = vec![0.0_f32; NUM_SAMPLES];
        let mut pcm1 = vec![0.0_f32; NUM_SAMPLES];
        for i in 0..NUM_SAMPLES {
            let amp0 = if i < 512 { 0.01 } else { 0.12 };
            let amp1 = if i < 256 { 0.12 } else { 0.02 };
            pcm0[i] = (2.0 * std::f32::consts::PI * 4_400.0 * i as f32 / 44_100.0).sin() * amp0;
            pcm1[i] = (2.0 * std::f32::consts::PI * 5_200.0 * i as f32 / 44_100.0).sin() * amp1;
        }

        assert_eq!(
            ProcessResult::LookAhead,
            encoder.process_frame(&mut pcm0, &ProcessMeta { channels: 1 })
        );
        assert_eq!(
            ProcessResult::Processed,
            encoder.process_frame(&mut pcm1, &ProcessMeta { channels: 1 })
        );

        let text = String::from_utf8(log_data.borrow().clone()).unwrap();
        assert!(text.contains("---\nframe: 0\n"));
        assert!(text.contains("channels:\n  - channel: 0\n"));
        assert!(text.contains("      - band: 0\n"));
        assert!(text.contains("gain_energy_scale:"));
    }
}
