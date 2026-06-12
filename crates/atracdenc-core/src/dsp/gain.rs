use std::marker::PhantomData;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct GainPoint {
    pub level: u32,
    pub location: u32,
}

pub trait GainParams {
    const GAIN_LEVEL: &'static [f32];
    const GAIN_INTERPOLATION: &'static [f32];
    const EXPONENT_OFFSET: i32;
    const GAIN_INTERPOLATION_POS_SHIFT: i32;
    const LOC_SCALE: u32;
    const LOC_SZ: u32;
    const MDCT_SZ: usize;
}

pub struct GainProcessor<P: GainParams>(PhantomData<P>);

impl<P: GainParams> GainProcessor<P> {
    pub fn get_gain_inc(level_idx_cur: u32) -> f32 {
        let inc_pos = P::EXPONENT_OFFSET - level_idx_cur as i32 + P::GAIN_INTERPOLATION_POS_SHIFT;
        P::GAIN_INTERPOLATION[inc_pos as usize]
    }

    pub fn get_gain_inc2(level_idx_cur: u32, level_idx_next: u32) -> f32 {
        let inc_pos =
            level_idx_next as i32 - level_idx_cur as i32 + P::GAIN_INTERPOLATION_POS_SHIFT;
        P::GAIN_INTERPOLATION[inc_pos as usize]
    }

    pub fn demodulate(
        gi_now: &[GainPoint],
        gi_next: &[GainPoint],
        out: &mut [f32],
        cur: &[f32],
        prev: &[f32],
    ) {
        let half = P::MDCT_SZ / 2;
        assert!(out.len() >= half && cur.len() >= half && prev.len() >= half);

        let mut pos = 0_usize;
        let scale = gi_next
            .first()
            .map(|p| P::GAIN_LEVEL[p.level as usize])
            .unwrap_or(1.0);

        for (i, point) in gi_now.iter().enumerate() {
            let last_pos = (point.location << P::LOC_SCALE) as usize;
            let mut level = P::GAIN_LEVEL[point.level as usize];
            let next_level = gi_now
                .get(i + 1)
                .map(|p| p.level as i32)
                .unwrap_or(P::EXPONENT_OFFSET);
            let inc_pos = next_level - point.level as i32 + P::GAIN_INTERPOLATION_POS_SHIFT;
            let gain_inc = P::GAIN_INTERPOLATION[inc_pos as usize];

            while pos < last_pos {
                out[pos] = (cur[pos] * scale + prev[pos]) * level;
                pos += 1;
            }
            while pos < last_pos + P::LOC_SZ as usize {
                out[pos] = (cur[pos] * scale + prev[pos]) * level;
                level *= gain_inc;
                pos += 1;
            }
        }

        while pos < half {
            out[pos] = cur[pos] * scale + prev[pos];
            pos += 1;
        }
    }

    pub fn modulate(gi_cur: &[GainPoint], buf_cur: &mut [f32], buf_next: &mut [f32]) {
        if gi_cur.is_empty() {
            return;
        }

        let half = P::MDCT_SZ / 2;
        assert!(buf_cur.len() >= half && buf_next.len() >= half);

        let mut pos = 0_usize;
        let scale = P::GAIN_LEVEL[gi_cur[0].level as usize];

        for (i, point) in gi_cur.iter().enumerate() {
            let last_pos = (point.location << P::LOC_SCALE) as usize;
            let mut level = P::GAIN_LEVEL[point.level as usize];
            let next_level = gi_cur
                .get(i + 1)
                .map(|p| p.level as i32)
                .unwrap_or(P::EXPONENT_OFFSET);
            let inc_pos = next_level - point.level as i32 + P::GAIN_INTERPOLATION_POS_SHIFT;
            let gain_inc = P::GAIN_INTERPOLATION[inc_pos as usize];

            while pos < last_pos {
                buf_cur[pos] /= scale;
                buf_next[pos] /= level;
                pos += 1;
            }
            while pos < last_pos + P::LOC_SZ as usize {
                buf_cur[pos] /= scale;
                buf_next[pos] /= level;
                level *= gain_inc;
                pos += 1;
            }
        }

        while pos < half {
            buf_cur[pos] /= scale;
            pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestParams;

    impl GainParams for TestParams {
        const GAIN_LEVEL: &'static [f32] = &[16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25, 0.125];
        const GAIN_INTERPOLATION: &'static [f32] = &[
            0.840_896_4,
            0.917_004_05,
            0.957_603_3,
            1.0,
            1.044_273_7,
            1.090_507_7,
            1.189_207_1,
        ];
        const EXPONENT_OFFSET: i32 = 4;
        const GAIN_INTERPOLATION_POS_SHIFT: i32 = 3;
        const LOC_SCALE: u32 = 2;
        const LOC_SZ: u32 = 4;
        const MDCT_SZ: usize = 32;
    }

    type Gp = GainProcessor<TestParams>;

    #[test]
    fn empty_gain_is_noop_for_modulate_and_overlap_for_demodulate() {
        let mut cur = vec![2.0; 16];
        let mut next = vec![3.0; 16];
        Gp::modulate(&[], &mut cur, &mut next);
        assert_eq!(vec![2.0; 16], cur);
        assert_eq!(vec![3.0; 16], next);

        let mut out = vec![0.0; 16];
        Gp::demodulate(&[], &[], &mut out, &next, &cur);
        assert_eq!(vec![5.0; 16], out);
    }

    #[test]
    fn modulate_demodulate_mirror_on_constant_region() {
        let gain = [GainPoint {
            level: 2,
            location: 2,
        }];
        let original_cur = (0..16).map(|i| i as f32 + 1.0).collect::<Vec<_>>();
        let original_next = (0..16).map(|i| 100.0 + i as f32).collect::<Vec<_>>();
        let mut cur = original_cur.clone();
        let mut next = original_next.clone();

        Gp::modulate(&gain, &mut cur, &mut next);
        let mut out = vec![0.0; 16];
        Gp::demodulate(&gain, &gain, &mut out, &next, &cur);

        let scale = TestParams::GAIN_LEVEL[gain[0].level as usize];
        for i in 0..8 {
            let expected = original_next[i] * scale + original_cur[i];
            assert!((out[i] - expected).abs() < 1.0e-4, "{i}");
        }
    }
}
