use super::scale::ScaledBlock;

pub const ENERGY_FLOOR: f32 = 1.0e-12;

pub fn analyze_scale_factor_spread(scaled_blocks: &[ScaledBlock]) -> f32 {
    let mean = scaled_blocks
        .iter()
        .map(|b| f32::from(b.scale_factor_index))
        .sum::<f32>()
        / scaled_blocks.len() as f32;
    let mut sigma = scaled_blocks
        .iter()
        .map(|b| {
            let t = f32::from(b.scale_factor_index) - mean;
            t * t
        })
        .sum::<f32>()
        / scaled_blocks.len() as f32;
    sigma = sigma.sqrt().min(14.0);
    sigma / 14.0
}

fn ath_formula_frank(mut freq: f32) -> f32 {
    #[rustfmt::skip]
    const TAB: [i16; 140] = [
        9669, 9669, 9626, 9512, 9353, 9113, 8882, 8676,
        8469, 8243, 7997, 7748, 7492, 7239, 7000, 6762,
        6529, 6302, 6084, 5900, 5717, 5534, 5351, 5167,
        5004, 4812, 4638, 4466, 4310, 4173, 4050, 3922,
        3723, 3577, 3451, 3281, 3132, 3036, 2902, 2760,
        2658, 2591, 2441, 2301, 2212, 2125, 2018, 1900,
        1770, 1682, 1594, 1512, 1430, 1341, 1260, 1198,
        1136, 1057, 998, 943, 887, 846, 744, 712,
        693, 668, 637, 606, 580, 555, 529, 502,
        475, 448, 422, 398, 375, 351, 327, 322,
        312, 301, 291, 268, 246, 215, 182, 146,
        107, 61, 13, -35, -96, -156, -179, -235,
        -295, -350, -401, -421, -446, -499, -532, -535,
        -513, -476, -431, -313, -179, 8, 203, 403,
        580, 736, 881, 1022, 1154, 1251, 1348, 1421,
        1479, 1399, 1285, 1193, 1287, 1519, 1914, 2369,
        3352, 4352, 5352, 6352, 7352, 8352, 9352, 9999,
        9999, 9999, 9999, 9999,
    ];

    freq = freq.clamp(10.0, 29_853.0);
    let freq_log = 40.0 * (0.1 * freq).log10();
    let index = freq_log as usize;
    0.01 * (f32::from(TAB[index]) * (1.0 + index as f32 - freq_log)
        + f32::from(TAB[index + 1]) * (freq_log - index as f32))
}

pub fn calc_ath(len: usize, sample_rate: u32) -> Vec<f32> {
    let mut res = vec![0.0; len];
    let mf = sample_rate as f32 / 2000.0;
    for (i, y) in res.iter_mut().enumerate() {
        let f = (i + 1) as f32 * mf / len as f32;
        let mut trh = ath_formula_frank(1000.0 * f) - 100.0;
        trh -= f * f * 0.015;
        *y = trh;
    }
    res
}

pub fn calc_spectral_flatness_per_bfu(
    mdct_energy: &[f32],
    specs_start: &[u32],
    specs_per_block: &[u32],
    num_bfu: usize,
    energy_floor: f32,
) -> Vec<f32> {
    let floor = energy_floor.max(1.0e-20);
    let mut flatness = vec![1.0; num_bfu];
    for bfu in 0..num_bfu {
        let start = specs_start[bfu] as usize;
        let len = specs_per_block[bfu] as usize;
        let end = start + len;
        assert!(end <= mdct_energy.len());
        if len == 0 {
            flatness[bfu] = 1.0;
            continue;
        }

        let mut arith_mean = 0.0_f64;
        let mut mean_log = 0.0_f64;
        for e in &mdct_energy[start..end] {
            let e = e.max(0.0);
            arith_mean += f64::from(e);
            mean_log += f64::from(e).max(f64::from(floor)).ln();
        }
        arith_mean /= len as f64;
        mean_log /= len as f64;

        if arith_mean <= f64::from(floor) {
            flatness[bfu] = 1.0;
            continue;
        }

        let ratio = mean_log.exp() / arith_mean;
        flatness[bfu] = ratio.clamp(0.0, 1.0) as f32;
    }
    flatness
}

pub fn track_loudness(prev_loud: f32, l0: f32, l1: f32) -> f32 {
    0.98 * prev_loud + 0.01 * (l0 + l1)
}

pub fn track_loudness_mono(prev_loud: f32, l: f32) -> f32 {
    0.98 * prev_loud + 0.02 * l
}

pub fn create_loudness_curve(sz: usize) -> Vec<f32> {
    let mut res = vec![0.0; sz];
    for (i, y) in res.iter_mut().enumerate() {
        let f = (i + 3) as f32 * 0.5 * 44_100.0 / sz as f32;
        let mut t = f.log10() - 3.5;
        t = -10.0 * t * t + 3.0 - f / 3000.0;
        *y = 10.0_f32.powf(0.1 * t);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectral_flatness_uniform_block() {
        let flatness = calc_spectral_flatness_per_bfu(&[4.0; 8], &[0], &[8], 1, ENERGY_FLOOR);
        assert_eq!(1, flatness.len());
        assert!((flatness[0] - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn spectral_flatness_impulse_maps_to_single_bfu() {
        let start = [0, 4, 8];
        let size = [4, 4, 4];
        let base_energy = vec![1.0; 12];

        for bfu in 0..3 {
            let mut mdct_energy = base_energy.clone();
            mdct_energy[start[bfu] as usize] = 32.0;
            let flatness =
                calc_spectral_flatness_per_bfu(&mdct_energy, &start, &size, 3, ENERGY_FLOOR);
            assert!(flatness[bfu] < 0.95);
            for (i, f) in flatness.iter().enumerate() {
                if i != bfu {
                    assert!((*f - 1.0).abs() < 1.0e-6);
                }
            }
        }
    }

    #[test]
    fn tone_like_energy_is_less_flat_than_noise_like_energy() {
        let start = [0];
        let size = [32];
        let mut tone = vec![1.0e-12; 32];
        tone[5] = 100.0;
        let noise = vec![1.0; 32];
        let tone_flat = calc_spectral_flatness_per_bfu(&tone, &start, &size, 1, ENERGY_FLOOR);
        let noise_flat = calc_spectral_flatness_per_bfu(&noise, &start, &size, 1, ENERGY_FLOOR);
        assert!(noise_flat[0] > tone_flat[0] + 0.08);
    }

    #[test]
    fn scale_factor_spread_and_loudness_tracking() {
        let blocks = [0, 28]
            .into_iter()
            .map(|idx| ScaledBlock {
                scale_factor_index: idx,
                values: Vec::new(),
                energy: 0.0,
            })
            .collect::<Vec<_>>();
        assert_eq!(1.0, analyze_scale_factor_spread(&blocks));
        assert!((track_loudness(10.0, 2.0, 4.0) - 9.86).abs() < 1.0e-6);
        assert!((track_loudness_mono(10.0, 4.0) - 9.88).abs() < 1.0e-6);
    }

    #[test]
    fn ath_and_loudness_curve_have_expected_shape() {
        let ath = calc_ath(128, 44_100);
        assert_eq!(128, ath.len());
        assert!(ath.iter().all(|x| x.is_finite()));

        let loud = create_loudness_curve(128);
        assert_eq!(128, loud.len());
        assert!(loud.iter().all(|x| x.is_finite() && *x >= 0.0));
    }
}
