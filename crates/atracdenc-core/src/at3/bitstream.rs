use crate::{
    at3::data::{
        BLOCK_SIZE_TAB, BLOCKS_PER_BAND, BfuAllocMode, CLC_LENGTH_TAB, ContainerParams,
        GainEnergyScale, HUFF_TABLES, MAX_BFUS, MAX_QUANT, MAX_SPECS, MAX_SPECS_PER_BLOCK,
        SPECS_PER_BLOCK, SPECS_START_LONG, TonalVal, mantissa_to_clc_idx, mantissas_to_vlc_index,
    },
    atrac::{
        psy::{analyze_scale_factor_spread, calc_ath},
        scale::{ScaledBlock, quant_mantissas},
    },
    bitstream::{BitStream, make_sign},
    container::CompressedOutput,
    dsp::gain::GainPoint,
    util::{div8_ceil, to_int},
};
use std::{io, sync::LazyLock};

pub const FIXED_BIT_ALLOC_TABLE: [u32; MAX_BFUS] = [
    6, 6, 5, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 0, 0, 0,
];
pub const LOSSY_NAQ_START: usize = 18;
pub const BOOST_NAQ_END: usize = 10;

static ATH: LazyLock<Vec<f32>> = LazyLock::new(|| {
    let ath_spec = calc_ath(1024, 44_100);
    let mut ath = Vec::with_capacity(MAX_BFUS);
    for band_num in 0..crate::at3::data::NUM_QMF {
        for block_num in BLOCKS_PER_BAND[band_num] as usize..BLOCKS_PER_BAND[band_num + 1] as usize
        {
            let spec_num_start = SPECS_START_LONG[block_num] as usize;
            let len = SPECS_PER_BLOCK[block_num] as usize;
            let min_ath = ath_spec[spec_num_start..spec_num_start + len]
                .iter()
                .copied()
                .fold(999.0_f32, f32::min);
            ath.push(10.0_f32.powf(0.1 * min_ath));
        }
    }
    ath
});

#[derive(Debug, Clone, PartialEq)]
pub struct TonalBlock {
    pub val: TonalVal,
    pub scaled_block: ScaledBlock,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SingleChannelElement {
    pub subband_info: Vec<Vec<GainPoint>>,
    pub tonal_blocks: Vec<TonalBlock>,
    pub scaled_blocks: Vec<ScaledBlock>,
    pub loudness: f32,
    pub gain_energy_scale: [GainEnergyScale; crate::at3::data::NUM_QMF],
}

impl SingleChannelElement {
    pub fn new(num_qmf: usize) -> Self {
        assert!((1..=crate::at3::data::NUM_QMF).contains(&num_qmf));
        Self {
            subband_info: vec![Vec::new(); num_qmf],
            tonal_blocks: Vec::new(),
            scaled_blocks: Vec::new(),
            loudness: 0.0,
            gain_energy_scale: [GainEnergyScale::default(); crate::at3::data::NUM_QMF],
        }
    }
}

pub struct Atrac3BitStreamWriter {
    params: ContainerParams,
    bfu_idx_const: u32,
    bfu_alloc_mode: BfuAllocMode,
    out_buffer: Vec<u8>,
}

impl Atrac3BitStreamWriter {
    pub fn new(params: ContainerParams, bfu_idx_const: u32) -> Self {
        Self::with_alloc_mode(params, bfu_idx_const, BfuAllocMode::Fast)
    }

    pub fn with_alloc_mode(
        params: ContainerParams,
        bfu_idx_const: u32,
        bfu_alloc_mode: BfuAllocMode,
    ) -> Self {
        Self {
            params,
            bfu_idx_const,
            bfu_alloc_mode,
            out_buffer: vec![0; params.frame_sz as usize],
        }
    }

    pub fn params(&self) -> ContainerParams {
        self.params
    }

    pub fn bfu_idx_const(&self) -> u32 {
        self.bfu_idx_const
    }

    pub fn bfu_alloc_mode(&self) -> BfuAllocMode {
        self.bfu_alloc_mode
    }

    pub fn frame_buffer(&self) -> &[u8] {
        &self.out_buffer
    }

    pub fn build_sound_unit_frame(
        &mut self,
        single_channel_elements: &[SingleChannelElement],
        loudness: f32,
    ) -> io::Result<Vec<u8>> {
        assert!(!single_channel_elements.is_empty());
        assert!(single_channel_elements.len() <= 2);
        assert!(!self.params.joint_stereo || single_channel_elements.len() == 2);

        self.out_buffer.clear();
        let half_frame_sz = self.params.frame_sz as usize >> 1;
        let mut bitstreams = [BitStream::new(), BitStream::new()];
        let mut bits_to_alloc = [-6_i32, -6_i32];

        for (channel, sce) in single_channel_elements.iter().enumerate() {
            let bitstream = &mut bitstreams[channel];
            if self.params.joint_stereo && channel == 1 {
                write_js_params(bitstream);
                bitstream.write(3, 2);
            } else {
                bitstream.write(0x28, 6);
            }

            assert!(!sce.subband_info.is_empty());
            assert!(sce.subband_info.len() <= crate::at3::data::NUM_QMF);
            bitstream.write((sce.subband_info.len() - 1) as u32, 2);

            for gain_points in &sce.subband_info {
                assert!(gain_points.len() < crate::at3::data::SubbandInfo::MAX_GAIN_POINTS_NUM);
                bitstream.write(gain_points.len() as u32, 3);
                for (idx, point) in gain_points.iter().enumerate() {
                    bitstream.write(point.level, 4);
                    bitstream.write(point.location, 5);
                    assert!(idx < 7);
                }
            }

            bits_to_alloc[channel] -= bitstream.size_in_bits() as i32;
        }

        let ms_bytes_shift = if self.params.joint_stereo {
            calc_ms_bytes_shift(
                self.params.frame_sz as u32,
                single_channel_elements,
                bits_to_alloc,
            )
        } else {
            0
        };

        bits_to_alloc[0] += 8 * (half_frame_sz as i32 + ms_bytes_shift);
        bits_to_alloc[1] += 8 * (half_frame_sz as i32 - ms_bytes_shift);

        for (channel, sce) in single_channel_elements.iter().enumerate() {
            let bitstream = &mut bitstreams[channel];
            let mut ctx = EncodeCtx {
                sce: Some(sce),
                target_bits: bits_to_alloc[channel].max(1) as u16,
                bfu_idx_const: self.bfu_idx_const,
                bfu_alloc_mode: self.bfu_alloc_mode,
                loudness,
                ..EncodeCtx::default()
            };
            configure_and_encode_specs(&mut ctx, bitstream)?;

            let target_len = if self.params.joint_stereo && channel == 1 {
                (half_frame_sz as i32 - ms_bytes_shift) as usize
            } else {
                (half_frame_sz as i32 + ms_bytes_shift) as usize
            };
            let mut channel_data = bitstream.bytes().to_vec();
            channel_data.resize(target_len, 0);
            if self.params.joint_stereo && channel == 1 {
                self.out_buffer.extend(channel_data.into_iter().rev());
            } else {
                self.out_buffer.extend(channel_data);
            }
        }

        if single_channel_elements.len() == 1 && !self.params.joint_stereo {
            assert_eq!(half_frame_sz, self.out_buffer.len());
            self.out_buffer.extend_from_within(..half_frame_sz);
        }

        self.out_buffer.resize(self.params.frame_sz as usize, 0);
        Ok(self.out_buffer.clone())
    }

    pub fn write_sound_unit(
        &mut self,
        output: &mut dyn CompressedOutput,
        single_channel_elements: &[SingleChannelElement],
        loudness: f32,
    ) -> io::Result<()> {
        let frame = self.build_sound_unit_frame(single_channel_elements, loudness)?;
        output.write_frame(&frame)
    }
}

pub fn sanitize_gain_energy_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

pub fn energy_scale_to_scale_factor_offset(energy_scale: f32) -> f32 {
    1.5 * sanitize_gain_energy_scale(energy_scale).log2()
}

pub fn calc_initial_num_bfu(bfu_idx_const: u32, target_bits: u16) -> u16 {
    let mut num_bfu = if bfu_idx_const != 0 {
        bfu_idx_const as u16
    } else {
        MAX_BFUS as u16
    };

    if target_bits < 101 {
        let mut lim = 1;
        if target_bits > 5 {
            lim = (target_bits - 5) / 3;
        }
        num_bfu = num_bfu.min(lim.max(1));
    }

    num_bfu.max(1)
}

pub fn check_bfus(num_bfu: &mut u16, precision_per_block: &[u32]) -> bool {
    assert!(*num_bfu > 0);
    assert_eq!(*num_bfu as usize, precision_per_block.len());
    let cur_last_bfu = *num_bfu - 1;
    if precision_per_block[cur_last_bfu as usize] == 0 {
        *num_bfu = cur_last_bfu;
        return true;
    }
    false
}

pub fn consider_energy_err(err: &[f32], bits: &mut [u32]) -> bool {
    assert!(err.len() >= bits.len());
    let mut adjusted = false;
    let lim = BOOST_NAQ_END.min(bits.len());
    for i in 0..lim {
        let e = err[i];
        if ((e > 0.0 && e < 0.7) || e > 1.2) & (bits[i] < 7) {
            bits[i] += 1;
            adjusted = true;
        }
    }
    adjusted
}

fn write_js_params(bs: &mut BitStream) {
    bs.write(0, 1);
    bs.write(7, 3);
    for _ in 0..4 {
        bs.write(3, 2);
    }
}

fn calc_ms_ratio(m_energy: f32, s_energy: f32) -> f32 {
    let total = s_energy + m_energy;
    if total > 0.0 {
        m_energy / total - 0.5
    } else {
        0.0
    }
}

fn calc_ms_bytes_shift(
    frame_sz: u32,
    elements: &[SingleChannelElement],
    bits_to_alloc: [i32; 2],
) -> i32 {
    let total_used_bits = 0 - bits_to_alloc[0] - bits_to_alloc[1];
    assert!(total_used_bits > 0);
    let max_allowed_shift = frame_sz as i32 / 2 - div8_ceil(total_used_bits as u32) as i32;

    if elements[1].scaled_blocks.is_empty() {
        max_allowed_shift
    } else {
        let ratio = calc_ms_ratio(elements[0].loudness, elements[1].loudness);
        to_int(frame_sz as f32 * ratio).clamp(-max_allowed_shift, max_allowed_shift)
    }
}

fn encode_tonal_components(
    sce: &SingleChannelElement,
    alloc_table: &[u32],
    bitstream: &mut BitStream,
) -> io::Result<u16> {
    let groups = group_tonal_components(&sce.tonal_blocks, alloc_table);
    let tcsgn = groups
        .iter()
        .map(|group| group.subgroup_starts.len())
        .sum::<usize>();
    assert!(tcsgn < 32);

    bitstream.write(tcsgn as u32, 5);
    let mut bits_used = 5_u16;

    if tcsgn == 0 {
        return Ok(bits_used);
    }

    bitstream.write(0, 2);
    bits_used += 2;

    let num_qmf_band = sce.subband_info.len();
    assert!(num_qmf_band <= crate::at3::data::NUM_QMF);
    assert_eq!(crate::at3::data::NUM_QMF, num_qmf_band);

    let mut tcgn_check = 0_u8;
    for (group_idx, group) in groups.iter().enumerate() {
        if group.ptrs.is_empty() {
            assert!(group.subgroup_starts.is_empty());
            continue;
        }
        assert!(!group.subgroup_starts.is_empty());

        for (subgroup, start_pos) in group.subgroup_starts.iter().copied().enumerate() {
            let end_pos = group
                .subgroup_starts
                .get(subgroup + 1)
                .copied()
                .unwrap_or(group.ptrs.len());
            assert!(end_pos > start_pos);
            let coded_values = group.ptrs[0].scaled_block.values.len();
            assert!(coded_values > 0 && coded_values < 8);

            let mut band_flags_c = [0_u8; 16];
            for tc in &group.ptrs[start_pos..end_pos] {
                assert_eq!(coded_values, tc.scaled_block.values.len());
                let spec_block = (tc.val.pos >> 6) as usize;
                assert!((spec_block >> 2) < num_qmf_band);
                band_flags_c[spec_block] += 1;
            }

            tcgn_check += 1;
            bits_used += num_qmf_band as u16;
            for qmf in 0..num_qmf_band {
                let active = band_flags_c[qmf * 4..qmf * 4 + 4]
                    .iter()
                    .any(|count| *count != 0);
                bitstream.write(u32::from(active), 1);
            }

            bits_used += 3;
            bitstream.write((coded_values - 1) as u32, 3);
            let quant_idx = group_idx >> 3;
            assert!(quant_idx > 1 && quant_idx < 8);
            bits_used += 3;
            bitstream.write(quant_idx as u32, 3);

            let mut last_pos = start_pos;
            let mut check_pos = 0;
            for spec_block in 0..16 {
                let qmf = spec_block >> 2;
                let active_qmf = band_flags_c[qmf * 4..qmf * 4 + 4]
                    .iter()
                    .any(|count| *count != 0);
                if !active_qmf {
                    continue;
                }

                let coded_components = band_flags_c[spec_block] as usize;
                assert!(coded_components < 8);
                bits_used += 3;
                bitstream.write(coded_components as u32, 3);

                for k in last_pos..last_pos + coded_components {
                    let tc = group.ptrs[k];
                    assert!(usize::from(tc.val.pos) >= spec_block * 64);
                    let rel_pos = usize::from(tc.val.pos) - spec_block * 64;
                    assert!(rel_pos < 64);
                    assert!(tc.scaled_block.scale_factor_index < 64);

                    bits_used += 6;
                    bitstream.write(u32::from(tc.scaled_block.scale_factor_index), 6);
                    bits_used += 6;
                    bitstream.write(rel_pos as u32, 6);

                    let mul = MAX_QUANT[quant_idx.min(7)];
                    let mut mantissas = [0_i32; 8];
                    for (idx, value) in tc.scaled_block.values.iter().enumerate() {
                        mantissas[idx] = to_int(*value * mul);
                    }
                    bits_used += vlc_encode(
                        quant_idx as u32,
                        &mantissas[..coded_values],
                        coded_values as u32,
                        Some(bitstream),
                    ) as u16;
                }
                last_pos += coded_components;
                check_pos = last_pos;
            }
            assert_eq!(end_pos, check_pos);
        }
    }
    assert_eq!(tcgn_check as usize, tcsgn);
    Ok(bits_used)
}

#[derive(Debug, Clone, Default)]
struct TonalComponentsSubGroup<'a> {
    subgroup_starts: Vec<usize>,
    ptrs: Vec<&'a TonalBlock>,
}

fn group_tonal_components<'a>(
    tonal_components: &'a [TonalBlock],
    alloc_table: &[u32],
) -> [TonalComponentsSubGroup<'a>; 64] {
    let mut groups: [TonalComponentsSubGroup<'a>; 64] =
        std::array::from_fn(|_| TonalComponentsSubGroup::default());

    for tc in tonal_components {
        assert!(!tc.scaled_block.values.is_empty() && tc.scaled_block.values.len() < 8);
        let bfu = tc.val.bfu as usize;
        if bfu >= alloc_table.len() {
            continue;
        }
        let quant = 2_u32.max((alloc_table[bfu] + 4).min(7)) as usize;
        groups[quant * 8 + tc.scaled_block.values.len()]
            .ptrs
            .push(tc);
    }

    for group in &mut groups {
        let mut cur_pos = 0;
        while cur_pos < group.ptrs.len() {
            let mut start_pos = cur_pos;
            group.subgroup_starts.push(cur_pos);
            let mut group_limiter = 0;
            loop {
                cur_pos += 1;
                if cur_pos == group.ptrs.len() {
                    break;
                }
                let base = group.ptrs[start_pos].val.pos & !63;
                if group.ptrs[cur_pos].val.pos - base < 64 {
                    group_limiter += 1;
                } else {
                    group_limiter = 0;
                    start_pos = cur_pos;
                }
                if group_limiter >= 7 {
                    break;
                }
            }
        }
    }

    groups
}

fn encode_specs(
    sce: &SingleChannelElement,
    bitstream: &mut BitStream,
    precision_per_block: &[u32],
    coding_mode: u8,
    mantissas: &[i32],
) -> io::Result<()> {
    encode_tonal_components(sce, precision_per_block, bitstream)?;
    assert!(!precision_per_block.is_empty());
    assert!(precision_per_block.len() <= MAX_BFUS);
    assert!(
        sce.scaled_blocks.len() >= precision_per_block.len()
            || precision_per_block.iter().all(|precision| *precision == 0)
    );

    bitstream.write((precision_per_block.len() - 1) as u32, 5);
    bitstream.write(u32::from(coding_mode), 1);

    for precision in precision_per_block {
        bitstream.write(*precision, 3);
    }
    for (i, precision) in precision_per_block.iter().enumerate() {
        if *precision != 0 {
            assert!(i < sce.scaled_blocks.len());
            bitstream.write(u32::from(sce.scaled_blocks[i].scale_factor_index), 6);
        }
    }
    for (i, precision) in precision_per_block.iter().enumerate() {
        if *precision == 0 {
            continue;
        }
        assert!(i < sce.scaled_blocks.len());
        let first = BLOCK_SIZE_TAB[i] as usize;
        let last = BLOCK_SIZE_TAB[i + 1] as usize;
        let block_size = (last - first) as u32;
        if coding_mode == 1 {
            clc_encode(
                *precision,
                &mantissas[first..last],
                block_size,
                Some(bitstream),
            );
        } else {
            vlc_encode(
                *precision,
                &mantissas[first..last],
                block_size,
                Some(bitstream),
            );
        }
    }
    Ok(())
}

fn configure_and_encode_specs(
    ctx: &mut EncodeCtx<'_>,
    bitstream: &mut BitStream,
) -> io::Result<()> {
    let sce = ctx.sce.expect("ATRAC3 encode context missing SCE");

    if sce.scaled_blocks.is_empty() {
        ctx.alloc_init_done = true;
        ctx.num_bfu = 1;
        ctx.spread = 0.0;
        ctx.coding_mode = 1;
        ctx.precision_per_block = vec![0];
        return encode_specs(
            sce,
            bitstream,
            &ctx.precision_per_block,
            ctx.coding_mode,
            &ctx.mantissas,
        );
    }

    ctx.spread = analyze_scale_factor_spread(&sce.scaled_blocks);
    ctx.num_bfu = calc_initial_num_bfu(ctx.bfu_idx_const, ctx.target_bits)
        .min(sce.scaled_blocks.len() as u16);
    ctx.alloc_init_done = true;

    let (mut best_precision, best_mode, best_mantissas) = match ctx.bfu_alloc_mode {
        BfuAllocMode::Fast => search_best_allocation(ctx, sce)?,
        BfuAllocMode::Parity => loop {
            let result = search_best_allocation(ctx, sce)?;
            if ctx.bfu_idx_const == 0 && result.0.len() > 1 {
                let mut num_bfu = result.0.len() as u16;
                if check_bfus(&mut num_bfu, &result.0) {
                    ctx.num_bfu = num_bfu;
                    continue;
                }
            }
            break result;
        },
    };

    if ctx.bfu_alloc_mode == BfuAllocMode::Fast && ctx.bfu_idx_const == 0 {
        while best_precision.len() > 1 {
            let mut num_bfu = best_precision.len() as u16;
            if check_bfus(&mut num_bfu, &best_precision) {
                best_precision.truncate(num_bfu as usize);
            } else {
                break;
            }
        }
    }

    ctx.precision_per_block = best_precision;
    ctx.coding_mode = best_mode;
    ctx.mantissas = best_mantissas;
    encode_specs(
        sce,
        bitstream,
        &ctx.precision_per_block,
        ctx.coding_mode,
        &ctx.mantissas,
    )
}

fn search_best_allocation(
    ctx: &mut EncodeCtx<'_>,
    sce: &SingleChannelElement,
) -> io::Result<(Vec<u32>, u8, [i32; MAX_SPECS])> {
    let mut lo = -8.0_f32;
    let mut hi = 20.0_f32;
    let mut best_precision = vec![0; ctx.num_bfu as usize];
    let mut best_mode = 1;
    let mut best_mantissas = [0; MAX_SPECS];
    let mut best_bits = u32::MAX;

    for _ in 0..64 {
        let shift = (lo + hi) / 2.0;
        let mut precision =
            calc_bits_allocation(sce, ctx.num_bfu as u32, ctx.spread, shift, ctx.loudness);
        ctx.energy_err = vec![0.0; precision.len()];

        let (mode, total_bits) = loop {
            let result = calc_specs_bits_consumption(
                sce,
                &precision,
                &mut ctx.mantissas,
                &mut ctx.energy_err,
            );
            if !consider_energy_err(&ctx.energy_err, &mut precision) {
                break result;
            }
        };
        let total_bits = total_bits
            + u32::from(encode_tonal_components(
                sce,
                &precision,
                &mut BitStream::new(),
            )?);

        if total_bits <= ctx.target_bits as u32 {
            best_precision = precision.clone();
            best_mode = mode;
            best_mantissas = ctx.mantissas;
            best_bits = total_bits;
            hi = shift - 0.01;
        } else {
            lo = shift + 0.01;
        }

        if hi <= lo {
            break;
        }
    }

    if best_bits == u32::MAX {
        let mut precision = vec![0; ctx.num_bfu as usize];
        let (mode, _) = calc_specs_bits_consumption(
            sce,
            &precision,
            &mut ctx.mantissas,
            &mut vec![0.0; precision.len()],
        );
        best_precision = std::mem::take(&mut precision);
        best_mode = mode;
        best_mantissas = ctx.mantissas;
    }

    Ok((best_precision, best_mode, best_mantissas))
}

pub fn calc_specs_bits_consumption(
    sce: &SingleChannelElement,
    precision_per_block: &[u32],
    mantissas: &mut [i32; MAX_SPECS],
    energy_err: &mut [f32],
) -> (u8, u32) {
    let num_blocks = precision_per_block.len();
    assert!(sce.scaled_blocks.len() >= num_blocks);
    assert!(energy_err.len() >= num_blocks);
    let bits_used = (num_blocks * 3) as u32;

    let mut clc_bits = 0;
    for i in 0..num_blocks {
        if precision_per_block[i] == 0 {
            continue;
        }
        clc_bits += 6;
        let first = BLOCK_SIZE_TAB[i];
        let last = BLOCK_SIZE_TAB[i + 1];
        let block_size = last - first;
        let mul = MAX_QUANT[precision_per_block[i].min(7) as usize];
        energy_err[i] = quant_mantissas(
            &sce.scaled_blocks[i].values,
            first,
            last,
            mul,
            i > LOSSY_NAQ_START,
            mantissas,
        );
        clc_bits += clc_encode(
            precision_per_block[i],
            &mantissas[first as usize..last as usize],
            block_size,
            None,
        );
    }

    let mut vlc_bits = 0;
    for i in 0..num_blocks {
        if precision_per_block[i] == 0 {
            continue;
        }
        vlc_bits += 6;
        let first = BLOCK_SIZE_TAB[i];
        let last = BLOCK_SIZE_TAB[i + 1];
        vlc_bits += vlc_encode(
            precision_per_block[i],
            &mantissas[first as usize..last as usize],
            last - first,
            None,
        );
    }

    let clc_mode = clc_bits <= vlc_bits;
    (
        u8::from(clc_mode),
        bits_used + if clc_mode { clc_bits } else { vlc_bits },
    )
}

pub fn calc_bits_allocation(
    sce: &SingleChannelElement,
    bfu_num: u32,
    spread: f32,
    shift: f32,
    loudness: f32,
) -> Vec<u32> {
    let bfu_num = bfu_num as usize;
    assert!(sce.scaled_blocks.len() >= bfu_num);
    let mut bits_per_block = vec![0; bfu_num];

    for (i, bits) in bits_per_block.iter_mut().enumerate() {
        let bfu_band = block_band(i);
        let gain_energy_scale = sanitize_gain_energy_scale(sce.gain_energy_scale[bfu_band].frame);
        let corrected_energy = sce.scaled_blocks[i].energy * gain_energy_scale;
        let ath = ATH[i] * loudness;

        if corrected_energy < ath {
            *bits = 0;
            continue;
        }

        let fix = FIXED_BIT_ALLOC_TABLE[i];
        let x = if i < 3 {
            2.8
        } else if i < 10 {
            2.6
        } else if i < 15 {
            3.3
        } else if i <= 20 {
            3.6
        } else if i <= 28 {
            4.2
        } else {
            6.0
        };
        let corrected_sfi = (f32::from(sce.scaled_blocks[i].scale_factor_index)
            + energy_scale_to_scale_factor_offset(gain_energy_scale))
        .clamp(0.0, 63.0);
        let tmp = (spread * (corrected_sfi / x) + (1.0 - spread) * fix as f32 - shift) as i32;
        *bits = if tmp > 7 {
            7
        } else if tmp < 0 {
            0
        } else if tmp == 0 {
            1
        } else {
            tmp as u32
        };
    }

    for tc in &sce.tonal_blocks {
        assert!(!tc.scaled_block.values.is_empty() && tc.scaled_block.values.len() < 8);
        let bfu = tc.val.bfu as usize;
        if bfu < bits_per_block.len() && bits_per_block[bfu] > 2 {
            bits_per_block[bfu] -= 1;
        }
    }

    bits_per_block
}

pub fn clc_encode(
    selector: u32,
    mantissas: &[i32],
    block_size: u32,
    mut bitstream: Option<&mut BitStream>,
) -> u32 {
    let num_bits = CLC_LENGTH_TAB[selector as usize];
    let bits_used = if selector > 1 {
        num_bits * block_size
    } else {
        num_bits * block_size / 2
    };

    if selector > 1 {
        for mantissa in mantissas.iter().take(block_size as usize) {
            if let Some(bs) = bitstream.as_deref_mut() {
                bs.write(make_sign(*mantissa, num_bits) as u32, num_bits as usize);
            }
        }
    } else {
        assert_eq!(4, num_bits);
        for i in 0..block_size as usize / 2 {
            let mut code = mantissa_to_clc_idx(mantissas[i * 2]) << 2;
            code |= mantissa_to_clc_idx(mantissas[i * 2 + 1]);
            if let Some(bs) = bitstream.as_deref_mut() {
                bs.write(code, num_bits as usize);
            }
        }
    }

    bits_used
}

pub fn vlc_encode(
    selector: u32,
    mantissas: &[i32],
    block_size: u32,
    mut bitstream: Option<&mut BitStream>,
) -> u32 {
    assert!(selector > 0);
    let huff_table = HUFF_TABLES[selector as usize - 1].table;
    let mut bits_used = 0;

    if selector > 1 {
        for mantissa in mantissas.iter().take(block_size as usize) {
            let mut huff_s = if *mantissa < 0 {
                ((-*mantissa as u32) << 1) | 1
            } else {
                (*mantissa as u32) << 1
            };
            huff_s = huff_s.saturating_sub(1);
            let entry = huff_table[huff_s as usize];
            bits_used += u32::from(entry.bits);
            if let Some(bs) = bitstream.as_deref_mut() {
                bs.write(u32::from(entry.code), entry.bits as usize);
            }
        }
    } else {
        assert_eq!(9, huff_table.len());
        for i in 0..block_size as usize / 2 {
            let huff_s = mantissas_to_vlc_index(mantissas[i * 2], mantissas[i * 2 + 1]);
            let entry = huff_table[huff_s as usize];
            bits_used += u32::from(entry.bits);
            if let Some(bs) = bitstream.as_deref_mut() {
                bs.write(u32::from(entry.code), entry.bits as usize);
            }
        }
    }

    bits_used
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct EncodeCtx<'a> {
    pub sce: Option<&'a SingleChannelElement>,
    pub target_bits: u16,
    pub bfu_idx_const: u32,
    pub bfu_alloc_mode: BfuAllocMode,
    pub loudness: f32,
    pub alloc_init_done: bool,
    pub spread: f32,
    pub num_bfu: u16,
    pub coding_mode: u8,
    pub precision_per_block: Vec<u32>,
    pub energy_err: Vec<f32>,
    pub mantissas: [i32; MAX_SPECS],
}

impl<'a> Default for EncodeCtx<'a> {
    fn default() -> Self {
        Self {
            sce: None,
            target_bits: 0,
            bfu_idx_const: 0,
            bfu_alloc_mode: BfuAllocMode::Fast,
            loudness: 0.0,
            alloc_init_done: false,
            spread: 0.0,
            num_bfu: 1,
            coding_mode: 1,
            precision_per_block: vec![0],
            energy_err: vec![0.0],
            mantissas: [0; MAX_SPECS],
        }
    }
}

#[allow(dead_code)]
pub(crate) fn block_band(block: usize) -> usize {
    let mut bfu_band = 0;
    for (band, &blocks) in BLOCKS_PER_BAND
        .iter()
        .enumerate()
        .skip(1)
        .take(crate::at3::data::NUM_QMF - 1)
    {
        if block >= blocks as usize {
            bfu_band = band;
        }
    }
    bfu_band
}

#[allow(dead_code)]
pub(crate) fn max_specs_per_block() -> usize {
    MAX_SPECS_PER_BLOCK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_energy_scale_helpers_match_log_domain_formula() {
        assert_eq!(1.0, sanitize_gain_energy_scale(f32::NAN));
        assert_eq!(1.0, sanitize_gain_energy_scale(-1.0));
        assert!((energy_scale_to_scale_factor_offset(4.0) - 3.0).abs() < 1.0e-6);
        assert!((energy_scale_to_scale_factor_offset(0.25) + 3.0).abs() < 1.0e-6);
    }

    #[test]
    fn initial_bfu_count_matches_budget_limit() {
        assert_eq!(32, calc_initial_num_bfu(0, 101));
        assert_eq!(16, calc_initial_num_bfu(16, 400));
        assert_eq!(1, calc_initial_num_bfu(0, 5));
        assert_eq!(5, calc_initial_num_bfu(0, 20));
        assert_eq!(5, calc_initial_num_bfu(8, 20));
    }

    #[test]
    fn clc_selector_one_packs_mantissa_pairs() {
        let mut bs = BitStream::new();
        let bits = clc_encode(1, &[-2, -1, 0, 1], 4, Some(&mut bs));
        assert_eq!(8, bits);
        assert_eq!(8, bs.size_in_bits());
        assert_eq!(0b1011, bs.read(4));
        assert_eq!(0b0001, bs.read(4));
    }

    #[test]
    fn vlc_selector_one_counts_and_writes_huffman_pairs() {
        let mut bs = BitStream::new();
        let bits = vlc_encode(1, &[-1, -1, 0, 0], 4, Some(&mut bs));
        assert_eq!(6, bits);
        assert_eq!(6, bs.size_in_bits());
        assert_eq!(0b1_1111, bs.read(5));
        assert_eq!(0, bs.read(1));
    }

    #[test]
    fn energy_error_boosts_low_blocks_only() {
        let err = vec![1.3; 12];
        let mut bits = vec![1; 12];
        assert!(consider_energy_err(&err, &mut bits));
        assert!(bits[..BOOST_NAQ_END].iter().all(|x| *x == 2));
        assert!(bits[BOOST_NAQ_END..].iter().all(|x| *x == 1));
    }

    #[test]
    fn check_bfus_trims_one_trailing_zero_bfu() {
        let mut num_bfu = 3;
        assert!(check_bfus(&mut num_bfu, &[4, 2, 0]));
        assert_eq!(2, num_bfu);

        let mut num_bfu = 3;
        assert!(!check_bfus(&mut num_bfu, &[4, 0, 1]));
        assert_eq!(3, num_bfu);
    }

    #[test]
    fn bitstream_writer_defaults_to_fast_bfu_allocation() {
        let writer = Atrac3BitStreamWriter::new(crate::at3::data::LP2, 0);
        assert_eq!(BfuAllocMode::Fast, writer.bfu_alloc_mode());
    }

    #[test]
    fn block_band_uses_atrac3_bfu_boundaries() {
        assert_eq!(0, block_band(0));
        assert_eq!(0, block_band(17));
        assert_eq!(1, block_band(18));
        assert_eq!(2, block_band(26));
        assert_eq!(3, block_band(30));
        assert_eq!(128, max_specs_per_block());
    }

    #[test]
    fn empty_lp2_mono_sound_unit_duplicates_first_channel() {
        let mut writer = Atrac3BitStreamWriter::new(crate::at3::data::LP2, 0);
        let frame = writer
            .build_sound_unit_frame(&[SingleChannelElement::new(4)], 1.0)
            .unwrap();
        let half = crate::at3::data::LP2.frame_sz as usize / 2;

        assert_eq!(crate::at3::data::LP2.frame_sz as usize, frame.len());
        assert_eq!(&frame[..half], &frame[half..]);
        assert_eq!(0xA3, frame[0]);
    }

    #[test]
    fn empty_lp4_joint_stereo_reverses_second_channel_region() {
        let mut writer = Atrac3BitStreamWriter::new(crate::at3::data::LP4, 0);
        let frame = writer
            .build_sound_unit_frame(
                &[SingleChannelElement::new(4), SingleChannelElement::new(4)],
                1.0,
            )
            .unwrap();

        assert_eq!(crate::at3::data::LP4.frame_sz as usize, frame.len());
        assert_eq!(0xA3, frame[0]);
        assert_eq!(0x7F, *frame.last().unwrap());
    }

    #[test]
    fn non_empty_scaled_blocks_encode_spectral_payload() {
        let mut writer = Atrac3BitStreamWriter::new(crate::at3::data::LP2, 0);
        let mut sce = SingleChannelElement::new(4);
        sce.scaled_blocks.push(ScaledBlock {
            scale_factor_index: 32,
            values: vec![0.5; 8],
            energy: 1.0,
        });

        let frame = writer.build_sound_unit_frame(&[sce], 1.0).unwrap();
        assert_eq!(crate::at3::data::LP2.frame_sz as usize, frame.len());
    }

    #[test]
    fn parity_bfu_allocation_encodes_valid_spectral_payload() {
        let mut writer =
            Atrac3BitStreamWriter::with_alloc_mode(crate::at3::data::LP2, 0, BfuAllocMode::Parity);
        let mut sce = SingleChannelElement::new(4);
        sce.scaled_blocks.push(ScaledBlock {
            scale_factor_index: 32,
            values: vec![0.5; 8],
            energy: 1.0,
        });

        let frame = writer.build_sound_unit_frame(&[sce], 1.0).unwrap();
        assert_eq!(crate::at3::data::LP2.frame_sz as usize, frame.len());
    }

    #[test]
    fn specs_bit_count_matches_dumped_bits() {
        let mut sce = SingleChannelElement::new(4);
        sce.scaled_blocks.push(ScaledBlock {
            scale_factor_index: 32,
            values: vec![0.5; 8],
            energy: 1.0,
        });
        let precision = [2];
        let mut mantissas = [0; MAX_SPECS];
        let mut energy_err = [0.0];
        let (mode, bits) =
            calc_specs_bits_consumption(&sce, &precision, &mut mantissas, &mut energy_err);
        let mut bs = BitStream::new();
        encode_specs(&sce, &mut bs, &precision, mode, &mantissas).unwrap();

        assert_eq!(11 + bits as usize, bs.size_in_bits());
        assert!(energy_err[0].is_finite());
    }

    #[test]
    fn tonal_blocks_encode_payload() {
        let mut writer = Atrac3BitStreamWriter::new(crate::at3::data::LP2, 0);
        let mut sce = SingleChannelElement::new(4);
        sce.tonal_blocks.push(TonalBlock {
            val: TonalVal {
                pos: 0,
                val: 0.0,
                bfu: 0,
            },
            scaled_block: ScaledBlock {
                scale_factor_index: 32,
                values: vec![0.5, -0.25],
                energy: 0.3125,
            },
        });

        let frame = writer.build_sound_unit_frame(&[sce], 1.0).unwrap();
        assert_eq!(crate::at3::data::LP2.frame_sz as usize, frame.len());
    }

    #[test]
    fn tonal_bit_count_matches_dumped_bits() {
        let mut sce = SingleChannelElement::new(4);
        sce.tonal_blocks.push(TonalBlock {
            val: TonalVal {
                pos: 65,
                val: 0.0,
                bfu: 0,
            },
            scaled_block: ScaledBlock {
                scale_factor_index: 31,
                values: vec![0.5, -0.25],
                energy: 0.3125,
            },
        });
        let alloc = [2_u32];
        let mut bs = BitStream::new();
        let bits = encode_tonal_components(&sce, &alloc, &mut bs).unwrap();
        assert_eq!(bits as usize, bs.size_in_bits());
        assert!(bits > 5);
    }

    #[test]
    fn empty_sound_unit_writes_through_compressed_output() {
        #[derive(Default)]
        struct Sink {
            frames: Vec<Vec<u8>>,
        }

        impl CompressedOutput for Sink {
            fn write_frame(&mut self, data: &[u8]) -> io::Result<()> {
                self.frames.push(data.to_vec());
                Ok(())
            }

            fn name(&self) -> &str {
                ""
            }

            fn channels(&self) -> usize {
                2
            }
        }

        let mut writer = Atrac3BitStreamWriter::new(crate::at3::data::LP2, 0);
        let mut sink = Sink::default();
        writer
            .write_sound_unit(&mut sink, &[SingleChannelElement::new(4)], 1.0)
            .unwrap();

        assert_eq!(1, sink.frames.len());
        assert_eq!(
            crate::at3::data::LP2.frame_sz as usize,
            sink.frames[0].len()
        );
    }
}
