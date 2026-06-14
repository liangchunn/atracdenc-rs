//! ATRAC3+ per-subband MDCT/IMDCT (256-point) with SINE/STEEP windows.
//!
//! Port of `atracdenc/src/atrac/at3p/at3p_mdct.{h,cpp}` (LGPL-2.1).
//!
//! Each of the 16 subbands carries 128 samples; the transform is a 256-point
//! MDCT with a 128-sample overlap history per subband. Odd subbands have their
//! 128 spectral coefficients reversed (frequency inversion).

use std::sync::LazyLock;

use crate::dsp::mdct::{Mdct, Midct};

static SINE_WIN_128: LazyLock<[f32; 128]> = LazyLock::new(|| {
    let mut w = [0.0f32; 128];
    for i in 0..128 {
        w[i] = 2.0 * ((i as f32 + 0.5) * (std::f32::consts::PI / (2.0 * 128.0))).sin();
    }
    w
});

static SINE_WIN_64: LazyLock<[f32; 64]> = LazyLock::new(|| {
    let mut w = [0.0f32; 64];
    for i in 0..64 {
        w[i] = 2.0 * ((i as f32 + 0.5) * (std::f32::consts::PI / (2.0 * 64.0))).sin();
    }
    w
});

/// Per-frame window-type bitmask over the 16 subbands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct At3pMdctWin {
    flags: u16,
}

impl At3pMdctWin {
    pub const SINE: At3pMdctWin = At3pMdctWin { flags: 0 };
    pub const STEEP: At3pMdctWin = At3pMdctWin { flags: u16::MAX };

    pub fn new() -> Self {
        Self { flags: 0 }
    }

    pub fn set_steep_win(&mut self, sb: usize) {
        self.flags |= 1 << sb;
    }

    pub fn is_all_sine(&self) -> bool {
        self.flags == 0
    }

    pub fn is_all_steep(&self, sb_num: u8) -> bool {
        let mask = (1u16 << sb_num).wrapping_sub(1);
        (self.flags & mask) == mask
    }

    pub fn is_sb_steep(&self, sb: u8) -> bool {
        self.flags & (1 << sb) != 0
    }

    pub fn flags(&self) -> u16 {
        self.flags
    }
}

fn swap_array(p: &mut [f32], len: usize) {
    let (mut i, mut j) = (0usize, len - 1);
    while i < len / 2 {
        p.swap(i, j);
        i += 1;
        j -= 1;
    }
}

/// History buffer for the forward MDCT (256 floats per subband).
pub type MdctHistBuf = [[f32; 256]; 16];

/// Forward per-subband MDCT.
pub struct At3pMdct {
    mdct: Mdct,
}

impl At3pMdct {
    pub fn new() -> Self {
        Self {
            // C: NMDCT::TMDCT<256> with default scale 1.0.
            mdct: Mdct::new(256, 1.0),
        }
    }

    /// `specs` holds 2048 output coefficients (16 × 128); `bands[b]` provides
    /// 128 input samples for subband `b`.
    pub fn do_mdct(
        &mut self,
        specs: &mut [f32],
        bands: &[&[f32]; 16],
        work: &mut MdctHistBuf,
        win: At3pMdctWin,
    ) {
        let sine128 = &*SINE_WIN_128;
        let sine64 = &*SINE_WIN_64;
        for b in 0..16 {
            let flag = 1u16 << b;
            let src = bands[b];
            let tmp = &mut work[b];

            if win.flags & flag != 0 {
                for i in 0..64 {
                    tmp[128 + i] = src[i] * 2.0;
                }
                for i in 0..64 {
                    tmp[160 + i] = sine64[63 - i] * src[32 + i];
                }
                for v in tmp[224..256].iter_mut() {
                    *v = 0.0;
                }
            } else {
                for i in 0..128 {
                    tmp[128 + i] = sine128[127 - i] * src[i];
                }
            }

            let sp = self.mdct.transform(tmp);
            let cur = &mut specs[b * 128..b * 128 + 128];
            cur.copy_from_slice(&sp[..128]);

            if b & 1 != 0 {
                swap_array(cur, 128);
            }

            if win.flags & flag != 0 {
                for v in tmp[0..32].iter_mut() {
                    *v = 0.0;
                }
                for i in 0..64 {
                    tmp[i + 32] = sine64[i] * src[i + 32];
                }
                for i in 0..32 {
                    tmp[i + 96] = src[i + 96] * 2.0;
                }
            } else {
                for i in 0..128 {
                    tmp[i] = sine128[i] * src[i];
                }
            }
        }
    }
}

impl Default for At3pMdct {
    fn default() -> Self {
        Self::new()
    }
}

/// History buffer for the inverse MDCT.
pub struct MidctHistBuf {
    pub buf: [[f32; 128]; 16],
    pub win: At3pMdctWin,
}

impl Default for MidctHistBuf {
    fn default() -> Self {
        Self {
            buf: [[0.0; 128]; 16],
            win: At3pMdctWin::new(),
        }
    }
}

/// Inverse per-subband MDCT (used by tests / round-trips).
pub struct At3pMidct {
    midct: Midct,
}

impl At3pMidct {
    pub fn new() -> Self {
        Self {
            // C: NMDCT::TMIDCT<256> with default scale 256.
            midct: Midct::with_default_scale(256),
        }
    }

    pub fn do_midct(
        &mut self,
        specs: &mut [f32],
        bands: &mut [&mut [f32]; 16],
        work: &mut MidctHistBuf,
        win: At3pMdctWin,
    ) {
        let sine128 = &*SINE_WIN_128;
        let sine64 = &*SINE_WIN_64;
        for b in 0..16 {
            let flag = 1u16 << b;
            let cur = &mut specs[b * 128..b * 128 + 128];

            if b & 1 != 0 {
                swap_array(cur, 128);
            }

            let mut inv = [0.0f32; 256];
            inv.copy_from_slice(self.midct.transform(cur));

            if work.win.flags & flag != 0 {
                for v in inv[0..32].iter_mut() {
                    *v = 0.0;
                }
                for j in 0..64 {
                    inv[j + 32] *= sine64[j];
                }
                for j in 96..128 {
                    inv[j] *= 2.0;
                }
            } else {
                for j in 0..128 {
                    inv[j] *= sine128[j];
                }
            }

            if win.flags & flag != 0 {
                for j in 128..160 {
                    inv[j] *= 2.0;
                }
                for j in 0..64 {
                    inv[223 - j] *= sine64[j];
                }
                for v in inv[224..256].iter_mut() {
                    *v = 0.0;
                }
            } else {
                for j in 0..128 {
                    inv[255 - j] *= sine128[j];
                }
            }

            let dst = &mut bands[b];
            let tmp = &work.buf[b];
            for j in 0..128 {
                dst[j] = inv[j] + tmp[j];
            }
            work.buf[b].copy_from_slice(&inv[128..256]);
        }
        work.win = win;
    }
}

impl Default for At3pMidct {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_one_block() {
        let zero = [0.0f32; 2048];
        let mut specs = [0.0f32; 2048];

        let mut mdct = At3pMdct::new();
        let mut buff: MdctHistBuf = [[0.0; 256]; 16];
        let bands: [&[f32]; 16] = std::array::from_fn(|i| &zero[i * 128..i * 128 + 128]);
        mdct.do_mdct(&mut specs, &bands, &mut buff, At3pMdctWin::SINE);
        for s in specs {
            assert!(s.abs() < 1e-10);
        }

        let mut midct = At3pMidct::new();
        let mut buff2 = MidctHistBuf::default();
        let mut out = [0.0f32; 2048];
        // Split out into 16 mutable band slices.
        let mut chunks: Vec<&mut [f32]> = out.chunks_mut(128).collect();
        let mut bands_m: [&mut [f32]; 16] = std::array::from_fn(|_| {
            // placeholder; replaced below
            &mut [][..]
        });
        for (i, c) in chunks.drain(..).enumerate() {
            bands_m[i] = c;
        }
        midct.do_midct(&mut specs, &mut bands_m, &mut buff2, At3pMdctWin::SINE);
        for v in out {
            assert!(v.abs() < 1e-10);
        }
    }

    fn dc_test(first: At3pMdctWin, second: At3pMdctWin) {
        let dc = [1.0f32; 2048];
        let mut specs = [0.0f32; 4096];

        {
            let mut mdct = At3pMdct::new();
            let mut buff: MdctHistBuf = [[0.0; 256]; 16];
            let bands: [&[f32]; 16] = std::array::from_fn(|i| &dc[i * 128..i * 128 + 128]);
            mdct.do_mdct(&mut specs[0..2048], &bands, &mut buff, first);
            mdct.do_mdct(&mut specs[2048..4096], &bands, &mut buff, second);
        }

        {
            let mut midct = At3pMidct::new();
            let mut buff = MidctHistBuf::default();
            let mut result = [0.0f32; 2048];

            // First frame.
            {
                let mut spec0 = [0.0f32; 2048];
                spec0.copy_from_slice(&specs[0..2048]);
                let mut chunks: Vec<&mut [f32]> = result.chunks_mut(128).collect();
                let mut bands_m: [&mut [f32]; 16] = std::array::from_fn(|_| &mut [][..]);
                for (i, c) in chunks.drain(..).enumerate() {
                    bands_m[i] = c;
                }
                midct.do_midct(&mut spec0, &mut bands_m, &mut buff, first);
            }
            // Second frame (overwrites result, completing TDAC).
            {
                let mut spec1 = [0.0f32; 2048];
                spec1.copy_from_slice(&specs[2048..4096]);
                let mut chunks: Vec<&mut [f32]> = result.chunks_mut(128).collect();
                let mut bands_m: [&mut [f32]; 16] = std::array::from_fn(|_| &mut [][..]);
                for (i, c) in chunks.drain(..).enumerate() {
                    bands_m[i] = c;
                }
                midct.do_midct(&mut spec1, &mut bands_m, &mut buff, second);
            }

            for v in result {
                assert!((v - 1.0).abs() < 1e-6, "got {v}");
            }
        }
    }

    #[test]
    fn dc_sine_win() {
        dc_test(At3pMdctWin::SINE, At3pMdctWin::SINE);
    }

    #[test]
    fn dc_steep_win() {
        dc_test(At3pMdctWin::STEEP, At3pMdctWin::STEEP);
    }

    #[test]
    fn dc_sine_steep_win() {
        dc_test(At3pMdctWin::SINE, At3pMdctWin::STEEP);
    }

    #[test]
    fn dc_steep_sine_win() {
        dc_test(At3pMdctWin::STEEP, At3pMdctWin::SINE);
    }
}
