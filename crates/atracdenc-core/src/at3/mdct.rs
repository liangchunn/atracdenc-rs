use crate::{
    at3::data::{
        At3GainParams, DECODE_WINDOW, ENCODE_WINDOW, GAIN_INTERPOLATION,
        GAIN_INTERPOLATION_POS_SHIFT, GAIN_LEVEL, GainEnergyScale, LOC_SCALE, LOC_SZ, MDCT_SZ,
        NUM_QMF, NUM_SAMPLES,
    },
    dsp::{
        gain::{GainPoint, GainProcessor},
        mdct::{Mdct, Midct},
    },
    util::get_first_set_bit,
};

pub type At3GainProcessor = GainProcessor<At3GainParams>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainEnergyAnalysis {
    pub scale: GainEnergyScale,
    pub next_overlap_scale: f32,
}

impl Default for GainEnergyAnalysis {
    fn default() -> Self {
        Self {
            scale: GainEnergyScale::default(),
            next_overlap_scale: 1.0,
        }
    }
}

pub fn relation_to_idx(mut x: f32) -> u16 {
    if x <= 0.5 {
        x = 1.0 / x.max(0.000_488_281_25);
        4 + get_first_set_bit(x.trunc() as u32)
    } else {
        x = x.min(16.0);
        4 - get_first_set_bit(x.trunc() as u32)
    }
}

pub struct Atrac3Mdct {
    mdct512: Mdct,
    midct512: Midct,
}

impl Atrac3Mdct {
    pub fn new() -> Self {
        Self {
            mdct512: Mdct::new(MDCT_SZ, 1.0),
            midct512: Midct::with_default_scale(MDCT_SZ),
        }
    }

    pub fn mdct(&mut self, specs: &mut [f32], bands: &mut [&mut [f32]; NUM_QMF]) {
        let empty: [&[GainPoint]; NUM_QMF] = [&[], &[], &[], &[]];
        self.mdct_with_gain(specs, bands, &empty);
    }

    pub fn mdct_with_gain(
        &mut self,
        specs: &mut [f32],
        bands: &mut [&mut [f32]; NUM_QMF],
        gain_points: &[&[GainPoint]; NUM_QMF],
    ) {
        let mut max_levels = [0.0; NUM_QMF];
        self.mdct_with_gain_and_max(specs, bands, &mut max_levels, gain_points);
    }

    pub fn mdct_with_gain_and_max(
        &mut self,
        specs: &mut [f32],
        bands: &mut [&mut [f32]; NUM_QMF],
        max_levels: &mut [f32; NUM_QMF],
        gain_points: &[&[GainPoint]; NUM_QMF],
    ) {
        assert!(specs.len() >= NUM_SAMPLES);

        for band in 0..NUM_QMF {
            let src_buff = &mut *bands[band];
            assert!(src_buff.len() >= MDCT_SZ);
            let cur_spec = &mut specs[band * 256..band * 256 + 256];
            let mut tmp = vec![0.0; MDCT_SZ];

            tmp[..256].copy_from_slice(&src_buff[..256]);
            At3GainProcessor::modulate(gain_points[band], &mut tmp[..256], &mut src_buff[256..512]);

            let mut max = 0.0_f32;
            for i in 0..256 {
                max = max.max(src_buff[256 + i].abs());
                src_buff[i] = ENCODE_WINDOW[i] * src_buff[256 + i];
                tmp[256 + i] = ENCODE_WINDOW[255 - i] * src_buff[256 + i];
            }

            cur_spec.copy_from_slice(self.mdct512.transform(&tmp));
            if band & 1 != 0 {
                cur_spec.reverse();
            }
            max_levels[band] = max;
        }
    }

    pub fn midct(&mut self, specs: &mut [f32], bands: &mut [&mut [f32]; NUM_QMF]) {
        let empty: [(&[GainPoint], &[GainPoint]); NUM_QMF] =
            [(&[], &[]), (&[], &[]), (&[], &[]), (&[], &[])];
        self.midct_with_gain(specs, bands, &empty);
    }

    pub fn midct_with_gain(
        &mut self,
        specs: &mut [f32],
        bands: &mut [&mut [f32]; NUM_QMF],
        gain_points: &[(&[GainPoint], &[GainPoint]); NUM_QMF],
    ) {
        assert!(specs.len() >= NUM_SAMPLES);

        for band in 0..NUM_QMF {
            let dst_buff = &mut *bands[band];
            assert!(dst_buff.len() >= MDCT_SZ);
            let cur_spec = &mut specs[band * 256..band * 256 + 256];

            if band & 1 != 0 {
                cur_spec.reverse();
            }

            let mut inv = self.midct512.transform(cur_spec).to_vec();
            for j in 0..256 {
                inv[j] *= 2.0 * DECODE_WINDOW[j];
                inv[511 - j] *= 2.0 * DECODE_WINDOW[j];
            }

            let (dst, prev_buff) = dst_buff.split_at_mut(256);
            let (gi_now, gi_next) = gain_points[band];
            if gi_now.is_empty() && gi_next.is_empty() {
                for j in 0..256 {
                    dst[j] = inv[j] + prev_buff[j];
                }
            } else {
                At3GainProcessor::demodulate(gi_now, gi_next, dst, &inv[..256], prev_buff);
            }
            prev_buff[..256].copy_from_slice(&inv[256..512]);
        }
    }

    pub fn make_gain_modulator_array(
        subband_info: &crate::at3::data::SubbandInfo,
    ) -> [&[GainPoint]; NUM_QMF] {
        let mut out = [&[][..], &[][..], &[][..], &[][..]];
        for (band, dst) in out.iter_mut().enumerate().take(subband_info.qmf_num()) {
            *dst = subband_info.gain_points(band);
        }
        out
    }

    pub fn calc_gain_energy_scale(
        prev_overlap: &[f32],
        cur_input: &[f32],
        gain_points: &[GainPoint],
        mut prev_overlap_scale: f32,
    ) -> GainEnergyAnalysis {
        assert!(prev_overlap.len() >= 256);
        assert!(cur_input.len() >= 256);

        if !prev_overlap_scale.is_finite() || prev_overlap_scale <= 0.0 {
            prev_overlap_scale = 1.0;
        }

        let prev_div = gain_points
            .first()
            .map(|point| GAIN_LEVEL[point.level as usize])
            .unwrap_or(1.0);

        let prev_stored_energy = prev_overlap[..256].iter().map(|x| x * x).sum::<f32>();
        let prev_original_energy = prev_stored_energy * prev_overlap_scale;
        let prev_modulated_energy = prev_stored_energy / (prev_div * prev_div);

        let sample_div = build_sample_divisors(gain_points);
        let mut cur_original_energy = 0.0;
        let mut cur_modulated_energy = 0.0;
        let mut next_original_energy = 0.0;
        let mut next_modulated_energy = 0.0;

        for i in 0..256 {
            let cur = cur_input[i];
            let modulated = cur / sample_div[i];
            let win_cur = ENCODE_WINDOW[255 - i];
            let win_next = ENCODE_WINDOW[i];
            let cur_win = cur * win_cur;
            let mod_cur_win = modulated * win_cur;
            let next_win = cur * win_next;
            let mod_next_win = modulated * win_next;
            cur_original_energy += cur_win * cur_win;
            cur_modulated_energy += mod_cur_win * mod_cur_win;
            next_original_energy += next_win * next_win;
            next_modulated_energy += mod_next_win * mod_next_win;
        }

        GainEnergyAnalysis {
            scale: GainEnergyScale {
                prev_half: safe_energy_scale(prev_original_energy, prev_modulated_energy),
                cur_half: safe_energy_scale(cur_original_energy, cur_modulated_energy),
                frame: safe_energy_scale(
                    prev_original_energy + cur_original_energy,
                    prev_modulated_energy + cur_modulated_energy,
                ),
            },
            next_overlap_scale: safe_energy_scale(next_original_energy, next_modulated_energy),
        }
    }
}

impl Default for Atrac3Mdct {
    fn default() -> Self {
        Self::new()
    }
}

fn safe_energy_scale(original_energy: f32, modulated_energy: f32) -> f32 {
    const ENERGY_EPS: f32 = 1.0e-20;
    if original_energy <= ENERGY_EPS
        || modulated_energy <= ENERGY_EPS
        || !original_energy.is_finite()
        || !modulated_energy.is_finite()
    {
        return 1.0;
    }

    let scale = original_energy / modulated_energy;
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

fn build_sample_divisors(points: &[GainPoint]) -> [f32; 256] {
    let mut out = [1.0; 256];
    let mut pos = 0_usize;

    for (i, point) in points.iter().enumerate() {
        let last_pos = (point.location << LOC_SCALE) as usize;
        let mut level = GAIN_LEVEL[point.level as usize];
        let next_level = points
            .get(i + 1)
            .map(|p| p.level as i32)
            .unwrap_or(crate::at3::data::EXPONENT_OFFSET);
        let inc_pos = next_level - point.level as i32 + GAIN_INTERPOLATION_POS_SHIFT;
        let gain_inc = GAIN_INTERPOLATION[inc_pos as usize];

        while pos < last_pos && pos < 256 {
            out[pos] = level;
            pos += 1;
        }
        while pos < last_pos + LOC_SZ as usize && pos < 256 {
            out[pos] = level;
            level *= gain_inc;
            pos += 1;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_signal(buf: &mut [f32], f: f32, a: f32) {
        for (i, sample) in buf.iter_mut().enumerate() {
            *sample = a * (std::f32::consts::FRAC_PI_2 * i as f32 * f).sin();
        }
    }

    fn assert_close_delayed(original: &[f32], decoded: &[f32], delay: usize, eps: f32) {
        for i in delay..original.len() {
            assert!(
                (original[i - delay] - decoded[i]).abs() <= eps,
                "{i}: {} != {}, eps {eps}",
                original[i - delay],
                decoded[i]
            );
        }
    }

    #[test]
    fn relation_to_idx_matches_cpp_cases() {
        assert_eq!(4, relation_to_idx(1.0));
        assert_eq!(4, relation_to_idx(1.8));
        assert_eq!(3, relation_to_idx(2.0));
        assert_eq!(3, relation_to_idx(3.5));
        assert_eq!(2, relation_to_idx(4.0));
        assert_eq!(1, relation_to_idx(8.0));
        assert_eq!(0, relation_to_idx(16.0));
        assert_eq!(0, relation_to_idx(9999.0));
        assert_eq!(4, relation_to_idx(0.8));
        assert_eq!(5, relation_to_idx(0.5));
        assert_eq!(6, relation_to_idx(0.25));
        assert_eq!(7, relation_to_idx(0.125));
        assert_eq!(8, relation_to_idx(0.0625));
        assert_eq!(9, relation_to_idx(0.03125));
        assert_eq!(10, relation_to_idx(0.015625));
        assert_eq!(13, relation_to_idx(0.001_953_125));
        assert_eq!(15, relation_to_idx(0.000_488_281_25));
        assert_eq!(15, relation_to_idx(0.000_000_488_281_3));
    }

    #[test]
    fn atrac3_window_matches_cpp_identity() {
        for i in 0..256 {
            let ha1 = ENCODE_WINDOW[i] / 2.0;
            let hs1 = DECODE_WINDOW[i];
            let hs2 = DECODE_WINDOW[255 - i];
            let res = hs1 / (hs1 * hs1 + hs2 * hs2);
            assert!((ha1 - res).abs() <= 2.0e-7, "{i}");
        }
    }

    #[test]
    fn atrac3_mdct_zero_block() {
        let mut mdct = Atrac3Mdct::new();
        let mut specs = vec![0.0; NUM_SAMPLES];
        let mut band0 = vec![0.0; MDCT_SZ];
        let mut band1 = vec![0.0; MDCT_SZ];
        let mut band2 = vec![0.0; MDCT_SZ];
        let mut band3 = vec![0.0; MDCT_SZ];
        let mut band_refs = [
            band0.as_mut_slice(),
            band1.as_mut_slice(),
            band2.as_mut_slice(),
            band3.as_mut_slice(),
        ];

        mdct.mdct(&mut specs, &mut band_refs);
        assert!(specs.iter().all(|x| x.abs() < 1.0e-10));

        mdct.midct(&mut specs, &mut band_refs);
        assert!(
            band0
                .iter()
                .chain(&band1)
                .chain(&band2)
                .chain(&band3)
                .all(|x| x.abs() < 1.0e-10)
        );
    }

    #[test]
    fn atrac3_mdct_sine_roundtrip_one_band() {
        let mut mdct = Atrac3Mdct::new();
        let mut signal = vec![0.0; 1024];
        let mut signal_res = vec![0.0; 1024];
        generate_signal(&mut signal, 0.25, 1.0);

        let mut enc_band0 = vec![0.0; MDCT_SZ];
        let mut enc_band1 = vec![0.0; MDCT_SZ];
        let mut enc_band2 = vec![0.0; MDCT_SZ];
        let mut enc_band3 = vec![0.0; MDCT_SZ];
        let mut dec_band0 = vec![0.0; MDCT_SZ];
        let mut dec_band1 = vec![0.0; MDCT_SZ];
        let mut dec_band2 = vec![0.0; MDCT_SZ];
        let mut dec_band3 = vec![0.0; MDCT_SZ];

        for pos in (0..signal.len()).step_by(256) {
            enc_band0[256..512].copy_from_slice(&signal[pos..pos + 256]);
            let mut specs = vec![0.0; NUM_SAMPLES];

            let mut enc_refs = [
                enc_band0.as_mut_slice(),
                enc_band1.as_mut_slice(),
                enc_band2.as_mut_slice(),
                enc_band3.as_mut_slice(),
            ];
            mdct.mdct(&mut specs, &mut enc_refs);

            let mut dec_refs = [
                dec_band0.as_mut_slice(),
                dec_band1.as_mut_slice(),
                dec_band2.as_mut_slice(),
                dec_band3.as_mut_slice(),
            ];
            mdct.midct(&mut specs, &mut dec_refs);
            signal_res[pos..pos + 256].copy_from_slice(&dec_band0[..256]);
        }

        assert_close_delayed(&signal, &signal_res, 256, 1.0e-3);
    }

    fn dc_gain_roundtrip(curve: &[GainPoint], eps: f32) {
        let mut mdct = Atrac3Mdct::new();
        let signal = vec![1.0; 2048];
        let mut signal_res = vec![0.0; 2048];

        let mut enc_band0 = vec![0.0; MDCT_SZ];
        let mut enc_band1 = vec![0.0; MDCT_SZ];
        let mut enc_band2 = vec![0.0; MDCT_SZ];
        let mut enc_band3 = vec![0.0; MDCT_SZ];
        let mut dec_band0 = vec![0.0; MDCT_SZ];
        let mut dec_band1 = vec![0.0; MDCT_SZ];
        let mut dec_band2 = vec![0.0; MDCT_SZ];
        let mut dec_band3 = vec![0.0; MDCT_SZ];

        let empty: [&[GainPoint]; NUM_QMF] = [&[], &[], &[], &[]];
        let gain: [&[GainPoint]; NUM_QMF] = [curve, &[], &[], &[]];
        let no_demod: [(&[GainPoint], &[GainPoint]); NUM_QMF] =
            [(&[], &[]), (&[], &[]), (&[], &[]), (&[], &[])];
        let demod_next: [(&[GainPoint], &[GainPoint]); NUM_QMF] =
            [(&[], curve), (&[], &[]), (&[], &[]), (&[], &[])];
        let demod_cur: [(&[GainPoint], &[GainPoint]); NUM_QMF] =
            [(curve, &[]), (&[], &[]), (&[], &[]), (&[], &[])];

        for pos in (0..signal.len()).step_by(256) {
            enc_band0[256..512].copy_from_slice(&signal[pos..pos + 256]);
            let mut specs = vec![0.0; NUM_SAMPLES];
            let gain_points = if pos == 1024 { &gain } else { &empty };

            let mut enc_refs = [
                enc_band0.as_mut_slice(),
                enc_band1.as_mut_slice(),
                enc_band2.as_mut_slice(),
                enc_band3.as_mut_slice(),
            ];
            mdct.mdct_with_gain(&mut specs, &mut enc_refs, gain_points);

            let demod = if pos == 1024 {
                &demod_next
            } else if pos == 1280 {
                &demod_cur
            } else {
                &no_demod
            };
            let mut dec_refs = [
                dec_band0.as_mut_slice(),
                dec_band1.as_mut_slice(),
                dec_band2.as_mut_slice(),
                dec_band3.as_mut_slice(),
            ];
            mdct.midct_with_gain(&mut specs, &mut dec_refs, demod);
            signal_res[pos..pos + 256].copy_from_slice(&dec_band0[..256]);
        }

        assert_close_delayed(&signal, &signal_res, 256, eps);
    }

    #[test]
    fn atrac3_mdct_gain_one_point_dc_roundtrip() {
        dc_gain_roundtrip(
            &[GainPoint {
                level: 3,
                location: 2,
            }],
            2.0e-6,
        );
    }

    #[test]
    fn gain_energy_scale_is_finite_and_tracks_no_gain_identity() {
        let prev = vec![0.25; 256];
        let cur = vec![0.5; 256];
        let analysis = Atrac3Mdct::calc_gain_energy_scale(&prev, &cur, &[], 1.0);

        assert!((analysis.scale.prev_half - 1.0).abs() < 1.0e-6);
        assert!((analysis.scale.cur_half - 1.0).abs() < 1.0e-6);
        assert!((analysis.scale.frame - 1.0).abs() < 1.0e-6);
        assert!((analysis.next_overlap_scale - 1.0).abs() < 1.0e-6);
    }
}
