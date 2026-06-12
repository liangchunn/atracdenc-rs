use crate::{
    at1::data::{BlockSizeMod, NUM_QMF, SINE_WINDOW},
    dsp::mdct::{Mdct, Midct},
    util::swap_array,
};

pub struct Atrac1Mdct {
    mdct512: Mdct,
    mdct256: Mdct,
    mdct64: Mdct,
    midct512: Midct,
    midct256: Midct,
    midct64: Midct,
}

impl Atrac1Mdct {
    pub fn new() -> Self {
        Self {
            mdct512: Mdct::new(512, 1.0),
            mdct256: Mdct::new(256, 0.5),
            mdct64: Mdct::new(64, 0.5),
            midct512: Midct::new(512, 1024.0),
            midct256: Midct::new(256, 512.0),
            midct64: Midct::new(64, 128.0),
        }
    }

    pub fn mdct(
        &mut self,
        specs: &mut [f32],
        low: &mut [f32],
        mid: &mut [f32],
        hi: &mut [f32],
        block_size: &BlockSizeMod,
    ) {
        assert!(specs.len() >= 512);
        assert!(low.len() >= 256);
        assert!(mid.len() >= 256);
        assert!(hi.len() >= 512);

        let mut pos = 0;
        for band in 0..NUM_QMF {
            let num_mdct_blocks = 1_usize << block_size.log_count[band];
            let src_buf = match band {
                0 => &mut *low,
                1 => &mut *mid,
                _ => &mut *hi,
            };
            let buf_sz = if band == 2 { 256 } else { 128 };
            let block_sz = if num_mdct_blocks == 1 { buf_sz } else { 32 };
            let win_start = if num_mdct_blocks == 1 {
                if band == 2 { 112 } else { 48 }
            } else {
                0
            };
            let multiple = if num_mdct_blocks != 1 && band == 2 {
                2.0
            } else {
                1.0
            };
            let mut tmp = vec![0.0; 512];
            let mut block_pos = 0;

            for _ in 0..num_mdct_blocks {
                tmp[win_start..win_start + 32].copy_from_slice(&src_buf[buf_sz..buf_sz + 32]);
                for i in 0..32 {
                    src_buf[buf_sz + i] = SINE_WINDOW[i] * src_buf[block_pos + block_sz - 32 + i];
                    src_buf[block_pos + block_sz - 32 + i] =
                        SINE_WINDOW[31 - i] * src_buf[block_pos + block_sz - 32 + i];
                }
                tmp[win_start + 32..win_start + 32 + block_sz]
                    .copy_from_slice(&src_buf[block_pos..block_pos + block_sz]);

                let sp = if num_mdct_blocks == 1 {
                    if band == 2 {
                        self.mdct512.transform(&tmp[..512]).to_vec()
                    } else {
                        self.mdct256.transform(&tmp[..256]).to_vec()
                    }
                } else {
                    self.mdct64.transform(&tmp[..64]).to_vec()
                };

                for (i, x) in sp.iter().enumerate() {
                    specs[block_pos + pos + i] = *x * multiple;
                }
                if band != 0 {
                    swap_array(&mut specs[block_pos + pos..block_pos + pos + sp.len()]);
                }
                block_pos += 32;
            }
            pos += buf_sz;
        }
    }

    pub fn imdct(
        &mut self,
        specs: &mut [f32],
        mode: &BlockSizeMod,
        low: &mut [f32],
        mid: &mut [f32],
        hi: &mut [f32],
    ) {
        assert!(specs.len() >= 512);
        assert!(low.len() >= 256);
        assert!(mid.len() >= 256);
        assert!(hi.len() >= 512);

        let mut pos = 0;
        for band in 0..NUM_QMF {
            let num_mdct_blocks = 1_usize << mode.log_count[band];
            let buf_sz = if band == 2 { 256 } else { 128 };
            let block_sz = if num_mdct_blocks == 1 { buf_sz } else { 32 };
            let dst_buf = match band {
                0 => &mut *low,
                1 => &mut *mid,
                _ => &mut *hi,
            };
            let mut inv_buf = vec![0.0; 512];
            let mut prev_buf = dst_buf[buf_sz * 2 - 16..buf_sz * 2].to_vec();
            let mut start = 0;

            for _ in 0..num_mdct_blocks {
                if band != 0 {
                    swap_array(&mut specs[pos..pos + block_sz]);
                }

                let inv = if num_mdct_blocks != 1 {
                    self.midct64.transform(&specs[pos..pos + 32]).to_vec()
                } else if buf_sz == 128 {
                    self.midct256.transform(&specs[pos..pos + 128]).to_vec()
                } else {
                    self.midct512.transform(&specs[pos..pos + 256]).to_vec()
                };

                let half = inv.len() / 2;
                let quarter = inv.len() / 4;
                inv_buf[start..start + half].copy_from_slice(&inv[quarter..quarter + half]);

                vector_fmul_window(
                    &mut dst_buf[start..start + 32],
                    &prev_buf,
                    &inv_buf[start..start + 32],
                );

                prev_buf.clear();
                prev_buf.extend_from_slice(&inv_buf[start + 16..start + 32]);
                start += block_sz;
                pos += block_sz;
            }

            if num_mdct_blocks == 1 {
                let copy_len = if band == 2 { 240 } else { 112 };
                dst_buf[32..32 + copy_len].copy_from_slice(&inv_buf[16..16 + copy_len]);
            }

            for j in 0..16 {
                dst_buf[buf_sz * 2 - 16 + j] = inv_buf[buf_sz - 16 + j];
            }
        }
    }
}

impl Default for Atrac1Mdct {
    fn default() -> Self {
        Self::new()
    }
}

fn vector_fmul_window(dst: &mut [f32], src0: &[f32], src1: &[f32]) {
    assert!(dst.len() >= 32 && src0.len() >= 16 && src1.len() >= 32);
    for (out_idx, i) in (-16..0).enumerate() {
        let j = (-i - 1) as usize;
        let s0 = src0[(16 + i) as usize];
        let s1 = src1[j];
        let wi = SINE_WINDOW[(16 + i) as usize];
        let wj = SINE_WINDOW[16 + j];
        dst[out_idx] = s0 * wj - s1 * wi;
        dst[16 + j] = s0 * wi + s1 * wj;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc_eps(magn: f32) -> f32 {
        magn * 10.0_f32.powf(-114.0 / 20.0)
    }

    fn check_result128(a: &[f32], b: &[f32]) {
        let m = a.iter().copied().fold(0.0, f32::max);
        let eps = calc_eps(m).max(1.0e-3);
        for i in 0..96 {
            let got = 4.0 * b[i + 32];
            assert!(
                (a[i] - got).abs() <= eps,
                "{i}: {} != {got}, eps {eps}",
                a[i]
            );
        }
    }

    fn check_result256(a: &[f32], b: &[f32]) {
        let m = a.iter().copied().fold(0.0, f32::max);
        let eps = calc_eps(m).max(1.0e-3);
        for i in 0..192 {
            let got = 2.0 * b[i + 32];
            assert!(
                (a[i] - got).abs() <= eps,
                "{i}: {} != {got}, eps {eps}",
                a[i]
            );
        }
    }

    #[test]
    fn atrac1_mdct_long_encode_decode() {
        let mut mdct = Atrac1Mdct::new();
        let mut low = vec![0.0; 256];
        let mut mid = vec![0.0; 256];
        let mut hi = vec![0.0; 512];
        let mut specs = vec![0.0; 1024];
        let mut low_res = vec![0.0; 256];
        let mut mid_res = vec![0.0; 256];
        let mut hi_res = vec![0.0; 512];

        for i in 0..128 {
            low[i] = i as f32;
            mid[i] = i as f32;
        }
        for (i, x) in hi.iter_mut().enumerate().take(256) {
            *x = i as f32;
        }

        let block_size = BlockSizeMod::new(false, false, false);
        mdct.mdct(&mut specs, &mut low, &mut mid, &mut hi, &block_size);
        mdct.imdct(
            &mut specs,
            &block_size,
            &mut low_res,
            &mut mid_res,
            &mut hi_res,
        );

        check_result128(&low, &low_res);
        check_result128(&mid, &mid_res);
        check_result256(&hi, &hi_res);
    }

    #[test]
    fn atrac1_mdct_short_encode_decode() {
        let mut mdct = Atrac1Mdct::new();
        let mut low = vec![0.0; 256];
        let mut mid = vec![0.0; 256];
        let mut hi = vec![0.0; 512];
        let mut specs = vec![0.0; 1024];
        let mut low_res = vec![0.0; 256];
        let mut mid_res = vec![0.0; 256];
        let mut hi_res = vec![0.0; 512];

        for i in 0..128 {
            low[i] = i as f32;
            mid[i] = i as f32;
        }
        for (i, x) in hi.iter_mut().enumerate().take(256) {
            *x = i as f32;
        }
        let low_copy = low.clone();
        let mid_copy = mid.clone();
        let hi_copy = hi.clone();

        let block_size = BlockSizeMod::new(true, true, true);
        mdct.mdct(&mut specs, &mut low, &mut mid, &mut hi, &block_size);
        mdct.imdct(
            &mut specs,
            &block_size,
            &mut low_res,
            &mut mid_res,
            &mut hi_res,
        );

        check_result128(&low_copy, &low_res);
        check_result128(&mid_copy, &mid_res);
        check_result256(&hi_copy, &hi_res);
    }
}
