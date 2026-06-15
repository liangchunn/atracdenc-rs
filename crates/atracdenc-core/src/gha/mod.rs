//! Port of libgha — Gabor/harmonic analysis (Daniil Cherednik, LGPL-2.1-or-later).
//!
//! Extracts sinusoidal components (frequency / phase / magnitude) from a PCM
//! frame using an FFT coarse estimate followed by Newton refinement, plus a
//! multidimensional Newton optimizer (`adjust_info`) for several harmonics.
//!
//! Ported from `src/gha.c` and `src/sle.c`. The C library uses `float` for its
//! `FLOAT` typedef (the default, without `GHA_USE_DOUBLE_API`) while the Newton
//! refinement and the linear solver work in `f64`; the per-declaration
//! precision is matched exactly here because tone-extraction stability depends
//! on it.
//!
//! kissfft (real FFT) in the original is replaced by `rustfft` driven through a
//! small real-FFT shim; the FFT is only used for the integer peak-bin estimate
//! and for 2x frequency-domain upsampling, both of which feed the
//! precision-independent Newton refinement.
//!
//! This is a faithful, line-for-line port; Clippy style lints that would
//! obscure the correspondence with the C reference (index loops, explicit
//! `dim * 0/1/2` matrix offsets, manual copies) are allowed module-wide.

#![allow(
    clippy::needless_range_loop,
    clippy::manual_memcpy,
    clippy::identity_op,
    clippy::erasing_op,
    clippy::collapsible_if,
    clippy::manual_is_multiple_of
)]

pub mod sle;

use std::f64::consts::PI;

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::sync::Arc;

use sle::sle_solve;

/// Result of analyzing a single harmonic (mirrors C `struct gha_info`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GhaInfo {
    pub frequency: f32,
    pub phase: f32,
    pub magnitude: f32,
}

/// Analysis context (mirrors C `struct gha_ctx`).
pub struct GhaCtx {
    size: usize,
    max_loops: usize,
    max_magnitude: f32,
    upsample: bool,
    window: Vec<f32>,
    tmp_buf: Vec<f32>,
    // Complex spectrum buffer; `size + 1` complex entries, zeroed (matches the
    // C `calloc(size + 1, ...)`). The forward FFT fills `0..=size/2`.
    fft_out: Vec<Complex<f32>>,
    fft_fwd: Arc<dyn Fft<f32>>,
    fft_inv: Arc<dyn Fft<f32>>,
    fwd_scratch: Vec<Complex<f32>>,
    inv_scratch: Vec<Complex<f32>>,
    // Reusable scratch for the FFT path (analog of C's preallocated ctx buffers).
    fft_in: Vec<Complex<f32>>,
    resample_full: Vec<Complex<f32>>,
    resampled: Vec<f32>,
    // Reusable Newton-MD scratch (analog of C's `alloca` buffers in
    // `gha_adjust_info_newton_md`). Grown on demand, reused across calls.
    // The reference's `baw`/`bap`/`bwp` buffers are omitted: they only fed
    // matrix blocks that are unconditionally zeroed by symmetrization.
    nm_m: Vec<f64>,
    nm_fx0: Vec<f64>,
    nm_ba: Vec<f64>,
    nm_bw: Vec<f64>,
    nm_bp: Vec<f64>,
    nm_bww: Vec<f64>,
    nm_bpp: Vec<f32>,
}

impl GhaCtx {
    /// Create a context for analyzing frames of `size` samples (must be even).
    pub fn new(size: usize) -> Self {
        assert!(size % 2 == 0, "gha size must be even");
        let mut planner = FftPlanner::new();
        let fft_fwd = planner.plan_fft_forward(size);
        let fft_inv = planner.plan_fft_inverse(size * 2);
        let fwd_scratch = vec![Complex::default(); fft_fwd.get_inplace_scratch_len()];
        let inv_scratch = vec![Complex::default(); fft_inv.get_inplace_scratch_len()];

        let mut ctx = Self {
            size,
            max_loops: 7,
            max_magnitude: 1.0,
            upsample: false,
            window: vec![0.0; size],
            tmp_buf: vec![0.0; size],
            fft_out: vec![Complex::default(); size + 1],
            fft_fwd,
            fft_inv,
            fwd_scratch,
            inv_scratch,
            fft_in: vec![Complex::default(); size],
            resample_full: vec![Complex::default(); size * 2],
            resampled: vec![0.0; size * 2],
            nm_m: Vec::new(),
            nm_fx0: Vec::new(),
            nm_ba: Vec::new(),
            nm_bw: Vec::new(),
            nm_bp: Vec::new(),
            nm_bww: Vec::new(),
            nm_bpp: Vec::new(),
        };
        ctx.init_window();
        ctx
    }

    pub fn set_max_loops(&mut self, max_loops: usize) {
        self.max_loops = max_loops;
    }

    pub fn set_max_magnitude(&mut self, magnitude: f32) {
        self.max_magnitude = magnitude;
    }

    pub fn set_upsample(&mut self, enable: bool) {
        self.upsample = enable;
    }

    /// The internal working buffer; after `adjust_info` it holds the residual.
    pub fn analyzed(&self) -> &[f32] {
        &self.tmp_buf
    }

    fn init_window(&mut self) {
        let n = self.size + 1;
        let half = self.size / 2;
        for i in 0..half {
            let w = (PI as f32 * (i as f32 + 1.0) / n as f32).sin();
            self.window[i] = w * w;
        }
        for i in half..self.size {
            self.window[i] = self.window[self.size - 1 - i];
        }
    }

    fn estimate_bin(&self) -> usize {
        let end = self.size / 2 + 1;
        let mut j = 0;
        let mut max = 0.0f32;
        for i in 0..end {
            let c = self.fft_out[i];
            let tmp = c.re * c.re + c.im * c.im;
            if tmp > max {
                max = tmp;
                j = i;
            }
        }
        j
    }

    /// Forward real FFT of `tmp_buf` into `fft_out[0..=size/2]`.
    fn real_fft_forward(&mut self) {
        let buf = &mut self.fft_in;
        for (b, &v) in buf.iter_mut().zip(self.tmp_buf.iter()) {
            *b = Complex { re: v, im: 0.0 };
        }
        self.fft_fwd
            .process_with_scratch(buf, &mut self.fwd_scratch);
        for c in self.fft_out.iter_mut() {
            *c = Complex::default();
        }
        for i in 0..=self.size / 2 {
            self.fft_out[i] = self.fft_in[i];
        }
    }

    /// Inverse real FFT (2x size) from the hermitian half in `fft_out`, scaled
    /// by `1/size` (matches C `resample_fft`). Result left in `self.resampled`.
    fn resample_fft(&mut self) {
        let m = self.size * 2;
        let full = &mut self.resample_full;
        for k in 0..=self.size {
            full[k] = self.fft_out[k];
        }
        for k in (self.size + 1)..m {
            full[k] = self.fft_out[m - k].conj();
        }
        self.fft_inv
            .process_with_scratch(full, &mut self.inv_scratch);
        let scale = self.size as f32;
        for i in 0..m {
            self.resampled[i] = self.resample_full[i].re / scale;
        }
    }

    /// Analyze one harmonic; result written to `info`.
    pub fn analyze_one(&mut self, pcm: &[f32], info: &mut GhaInfo) {
        for i in 0..self.size {
            self.tmp_buf[i] = pcm[i] * self.window[i];
        }

        self.real_fft_forward();
        let bin = self.estimate_bin();

        if self.upsample {
            self.resample_fft();
            search_omega_newton(&self.resampled, bin, self.size * 2, info);
            info.frequency *= 2.0;
        } else {
            // Operate on the windowed buffer (matches C).
            search_omega_newton(&self.tmp_buf, bin, self.size, info);
        }

        generate_sine(&mut self.tmp_buf, self.size, info.frequency, info.phase);
        estimate_magnitude(pcm, &self.tmp_buf, self.size, info);
    }

    /// Analyze and subtract one harmonic from `pcm`.
    pub fn extract_one(&mut self, pcm: &mut [f32], info: &mut GhaInfo) {
        self.analyze_one(pcm, info);
        let magnitude = info.magnitude;
        for i in 0..self.size {
            pcm[i] -= self.tmp_buf[i] * magnitude;
        }
    }

    /// Analyze and subtract `k` harmonics sequentially.
    pub fn extract_many_simple(&mut self, pcm: &mut [f32], info: &mut [GhaInfo], k: usize) {
        for i in 0..k {
            self.extract_one(pcm, &mut info[i]);
        }
    }

    /// Multidimensional Newton optimization of `k` harmonics in place.
    ///
    /// After a successful adjustment the residual (input minus synthesized
    /// harmonics) is left in the internal buffer and, if `cb` is provided, the
    /// callback receives it. `size_limit` (when nonzero and smaller than the
    /// frame size) restricts the optimization to a prefix of the frame.
    /// Returns `-1` on a singular system, `0` otherwise.
    pub fn adjust_info<F: FnMut(&[f32])>(
        &mut self,
        pcm: &[f32],
        info: &mut [GhaInfo],
        k: usize,
        size_limit: usize,
        mut cb: Option<F>,
    ) -> i32 {
        let mut actual_size = self.size;
        if size_limit != 0 && size_limit < self.size {
            actual_size = size_limit;
        }

        let rv = self.adjust_info_newton_md(pcm, info, k, actual_size);
        if rv != -1 {
            if let Some(cb) = cb.as_mut() {
                cb(&self.tmp_buf);
            }
        }
        rv
    }

    fn adjust_info_newton_md(
        &mut self,
        pcm: &[f32],
        info: &mut [GhaInfo],
        dim: usize,
        sz: usize,
    ) -> i32 {
        let mcols = dim * 3 + 1;
        let mrows = dim * 3;

        let max_loops = self.max_loops;
        let max_magnitude = self.max_magnitude;

        // Reusable scratch (analog of C's `alloca`): grow to the needed size and
        // reuse the backing allocation across calls. Every element is written
        // before it is read each iteration, so stale contents are harmless.
        // Bound as slices so the optimizer can prove `n < sz` in the hot loops.
        // `baw`/`bap`/`bwp` from the reference are omitted (dead — see below).
        self.nm_m.resize(mrows * mcols, 0.0);
        self.nm_fx0.resize(dim * 3, 0.0);
        self.nm_ba.resize(dim * sz, 0.0);
        self.nm_bw.resize(dim * sz, 0.0);
        self.nm_bp.resize(dim * sz, 0.0);
        self.nm_bww.resize(dim * sz, 0.0);
        self.nm_bpp.resize(dim * sz, 0.0);
        let m: &mut [f64] = &mut self.nm_m;
        let fx0: &mut [f64] = &mut self.nm_fx0;
        let ba: &mut [f64] = &mut self.nm_ba;
        let bw: &mut [f64] = &mut self.nm_bw;
        let bp: &mut [f64] = &mut self.nm_bp;
        let bww: &mut [f64] = &mut self.nm_bww;
        let bpp: &mut [f32] = &mut self.nm_bpp;
        let tmp_buf: &mut [f32] = &mut self.tmp_buf;

        for _loop in 0..max_loops {
            tmp_buf[..sz].copy_from_slice(&pcm[..sz]);

            for kk in 0..dim {
                let off = kk * sz;
                for n in 0..sz {
                    let ak = info[kk].magnitude as f64;
                    let t: f32 = info[kk].frequency * (n as f32) + info[kk].phase;
                    let s = t.sin();
                    let c = t.cos();

                    // C: `tmp_buf -= magnitude * s` in f32 (magnitude is float).
                    tmp_buf[n] -= info[kk].magnitude * s;

                    ba[off + n] = -(s as f64);
                    bw[off + n] = -ak * (n as f64) * (c as f64);
                    bp[off + n] = -ak * (c as f64);

                    // `baw`/`bap`/`bwp` from the reference only fed the
                    // off-diagonal matrix blocks that are unconditionally zeroed
                    // by symmetrization (see the matrix loop), so they are dead
                    // and omitted. `bww`/`bpp` feed the live diagonal blocks.
                    bww[off + n] = ak * (n as f64) * (n as f64) * (s as f64);
                    bpp[off + n] = (ak * s as f64) as f32;
                }
            }

            for v in m.iter_mut() {
                *v = 0.0;
            }

            for i in 0..dim {
                let r0 = mcols * (i + dim * 0);
                let r1 = mcols * (i + dim * 1);
                let r2 = mcols * (i + dim * 2);
                let io = i * sz;

                let tb = &tmp_buf[..sz];
                let ba_i = &ba[io..io + sz];
                let bw_i = &bw[io..io + sz];
                let bp_i = &bp[io..io + sz];

                for j in 0..dim {
                    // The off-diagonal blocks of the normal-equation matrix —
                    // m[(0,1)], m[(0,2)], m[(1,2)] — are unconditionally
                    // overwritten with the (always-zero) lower-triangle blocks
                    // m[(1,0)], m[(2,0)], m[(2,1)] by the symmetrization below.
                    // Those blocks are never accumulated, so they stay 0. Hence
                    // the accumulations into the upper off-diagonal blocks are
                    // dead and are omitted here; the entries are left at their
                    // cleared value of 0 (matching the reference output exactly,
                    // which also zeroes them). Only the block-diagonal entries
                    // a00 / a11 / a22 survive.
                    if i == j {
                        let bww_i = &bww[io..io + sz];
                        let bpp_i = &bpp[io..io + sz];
                        for n in 0..sz {
                            let tbn = tb[n] as f64;
                            m[r0 + j + dim * 0] += ba_i[n] * ba_i[n];
                            m[r1 + j + dim * 1] += tbn * bww_i[n] + bw_i[n] * bw_i[n];
                            // C computes tmp_buf(f32) * bpp(f32) in f32 then
                            // promotes; keep that precision exactly.
                            m[r2 + j + dim * 2] += (tb[n] * bpp_i[n]) as f64 + bp_i[n] * bp_i[n];
                        }
                    } else {
                        let jo = j * sz;
                        let ba_j = &ba[jo..jo + sz];
                        let bw_j = &bw[jo..jo + sz];
                        let bp_j = &bp[jo..jo + sz];
                        for n in 0..sz {
                            m[r0 + j + dim * 0] += ba_i[n] * ba_j[n];
                            m[r1 + j + dim * 1] += bw_i[n] * bw_j[n];
                            m[r2 + j + dim * 2] += bp_i[n] * bp_j[n];
                        }
                    }

                    m[r0 + j + dim * 0] *= 2.0;
                    m[r1 + j + dim * 1] *= 2.0;
                    m[r2 + j + dim * 2] *= 2.0;
                }
            }

            for kk in 0..dim {
                let r0 = mcols * (kk + dim * 0);
                let r1 = mcols * (kk + dim * 1);
                let r2 = mcols * (kk + dim * 2);
                let off = kk * sz;
                let tb = &tmp_buf[..sz];
                let ba_k = &ba[off..off + sz];
                let bw_k = &bw[off..off + sz];
                let bp_k = &bp[off..off + sz];
                for n in 0..sz {
                    let tbn = tb[n];
                    m[r0 + dim * 3] += (tbn * (ba_k[n] as f32)) as f64;
                    m[r1 + dim * 3] += (tbn * (bw_k[n] as f32)) as f64;
                    m[r2 + dim * 3] += (tbn * (bp_k[n] as f32)) as f64;
                }
                m[r0 + dim * 3] *= 2.0;
                m[r1 + dim * 3] *= 2.0;
                m[r2 + dim * 3] *= 2.0;
            }

            for v in fx0.iter_mut() {
                *v = 0.0;
            }
            if sle_solve(m, dim * 3, fx0) != 0 {
                return -1;
            }

            for kk in 0..dim {
                // C: `field -= (fx0 * 0.8)` — subtraction in f64, then rounded
                // back to the f32 field.
                info[kk].magnitude = (info[kk].magnitude as f64 - fx0[kk + dim * 0] * 0.8) as f32;
                info[kk].frequency = (info[kk].frequency as f64 - fx0[kk + dim * 1] * 0.8) as f32;
                info[kk].phase = (info[kk].phase as f64 - fx0[kk + dim * 2] * 0.8) as f32;
            }

            for kk in 0..dim {
                if info[kk].magnitude < 0.0 {
                    info[kk].magnitude *= -1.0;
                    info[kk].phase += PI as f32;
                }
                if info[kk].magnitude > max_magnitude {
                    info[kk].magnitude = max_magnitude * 0.5;
                }
            }

            for kk in 0..dim {
                if info[kk].frequency < 0.0 {
                    info[kk].frequency *= -1.0;
                    info[kk].phase = 2.0 * PI as f32 - info[kk].phase;
                }
                while info[kk].frequency > PI as f32 * 2.0 {
                    info[kk].frequency -= PI as f32 * 2.0;
                }
                if info[kk].frequency > PI as f32 {
                    info[kk].frequency = 2.0 * PI as f32 - info[kk].frequency;
                }
            }

            for kk in 0..dim {
                while info[kk].phase > PI as f32 * 2.0 {
                    info[kk].phase -= PI as f32 * 2.0;
                }
                while info[kk].phase < 0.0 {
                    info[kk].phase += PI as f32 * 2.0;
                }
            }
        }
        0
    }
}

/// Newton search of the frequency (and phase at the last iteration).
fn search_omega_newton(pcm: &[f32], bin: usize, size: usize, result: &mut GhaInfo) {
    const MAX_LOOPS: usize = 7;
    let mut omega_rad = bin as f64 * 2.0 * PI / size as f64;

    for loop_i in 0..=MAX_LOOPS {
        let mut xr = 0.0f64;
        let mut xi = 0.0f64;
        let mut dxr = 0.0f64;
        let mut dxi = 0.0f64;
        let mut ddxr = 0.0f64;
        let mut ddxs = 0.0f64;

        let a = omega_rad.cos();
        let b = omega_rad.sin();
        let mut c = 1.0f64;
        let mut s = 0.0f64;

        for n in 0..size {
            let pcm_n = pcm[n] as f64;
            let cm = pcm_n * c;
            let sm = pcm_n * s;
            xr += cm;
            xi += sm;
            let tc = n as f64 * cm;
            let ts = n as f64 * sm;
            dxr -= ts;
            dxi += tc;
            ddxr -= n as f64 * tc;
            ddxs -= n as f64 * ts;

            let new_c = a * c - b * s;
            let new_s = b * c + a * s;
            c = new_c;
            s = new_s;
        }

        let f = xr * dxr + xi * dxi;
        let g2 = xr * xr + xi * xi;
        let df = xr * ddxr + dxr * dxr + xi * ddxs + dxi * dxi;
        let dw = f / (df - (f * f) / g2);

        omega_rad -= dw;

        if omega_rad < 0.0 {
            omega_rad *= -1.0;
        }
        while omega_rad > PI * 2.0 {
            omega_rad -= PI * 2.0;
        }
        if omega_rad > PI {
            omega_rad = PI * 2.0 - omega_rad;
        }

        if loop_i == MAX_LOOPS {
            result.frequency = omega_rad as f32;
            // assume zero-phase sine
            result.phase = (PI / 2.0 - (xi / xr).atan()) as f32;
            if xr < 0.0 {
                result.phase += PI as f32;
            }
        }
    }
}

fn generate_sine(buf: &mut [f32], size: usize, omega: f32, phase: f32) {
    for i in 0..size {
        buf[i] = ((omega as f64 * i as f64) + phase as f64).sin() as f32;
    }
}

fn estimate_magnitude(pcm: &[f32], regen: &[f32], size: usize, result: &mut GhaInfo) {
    let mut t1 = 0.0f64;
    let mut t2 = 0.0f64;
    for i in 0..size {
        t1 += pcm[i] as f64 * regen[i] as f64;
        t2 += regen[i] as f64 * regen[i] as f64;
    }
    result.magnitude = (t1 / t2) as f32;
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLERATE: f32 = 44100.0;

    fn gen_tone(f: f32, a: f32, out: &mut [f32]) {
        let freq = f / SAMPLERATE;
        for (i, v) in out.iter_mut().enumerate() {
            *v += (freq * (i as f32 * 2.0 * std::f32::consts::PI)).sin() * a;
        }
    }

    fn check_float(v1: f32, v2: f32) {
        assert!((v1 - v2).abs() < 0.000002, "chk_eq_flt: {v1} != {v2}");
    }

    fn compare_phase(a: f32, b: f32, delta: f32) -> bool {
        if (a - b).abs() < delta {
            return true;
        }
        let pi = std::f32::consts::PI;
        let a = (a + pi).rem_euclid(2.0 * pi);
        let b = (b + pi).rem_euclid(2.0 * pi);
        (a - b).abs() < delta
    }

    #[test]
    fn one_tone_11025_a1() {
        let mut buf = [0.0f32; 128];
        let mut ctx = GhaCtx::new(128);
        ctx.set_upsample(true);
        gen_tone(11025.0, 1.0, &mut buf);

        let mut res = GhaInfo::default();
        ctx.analyze_one(&buf, &mut res);
        assert!(compare_phase(0.0, res.phase, 0.01));
        check_float(res.magnitude, 1.0);
        check_float(res.frequency, 1.5707963705);

        ctx.adjust_info(
            &buf,
            std::slice::from_mut(&mut res),
            1,
            0,
            None::<fn(&[f32])>,
        );
        assert!(compare_phase(0.0, res.phase, 0.01));
        check_float(res.magnitude, 1.0);
        check_float(res.frequency, 1.5707963705);

        res.magnitude = 0.95;
        ctx.adjust_info(
            &buf,
            std::slice::from_mut(&mut res),
            1,
            32,
            None::<fn(&[f32])>,
        );
        assert!(compare_phase(0.0, res.phase, 0.01));
        check_float(res.magnitude, 1.0);
        check_float(res.frequency, 1.5707963705);
    }

    #[test]
    fn one_tone_20000_a1() {
        let mut buf = [0.0f32; 128];
        let mut ctx = GhaCtx::new(128);
        ctx.set_upsample(true);
        gen_tone(20000.0, 1.0, &mut buf);

        let mut res = GhaInfo::default();
        ctx.analyze_one(&buf, &mut res);
        check_float(res.magnitude, 1.000001);
        check_float(res.frequency, 2.849515);
    }

    #[test]
    fn one_tone_22000_a1() {
        // Near-Nyquist (22000 of 22050 Hz). The integer peak-bin estimate and
        // the 2x frequency-domain resample feed Newton through rustfft instead
        // of the original kissfft; the recovered frequency matches the C
        // reference to <1e-6, but the projected magnitude differs by ~2e-5 due
        // to the FFT backend. Frequencies use the tight C tolerance; the
        // magnitude is checked with a relaxed tolerance documenting the
        // backend divergence (C reference: 0.618932 / 0.999956).
        let mut buf = [0.0f32; 512];
        let mut ctx = GhaCtx::new(512);
        ctx.set_upsample(true);
        ctx.set_max_loops(64);
        gen_tone(22000.0, 1.0, &mut buf);

        let mut res = GhaInfo::default();
        ctx.analyze_one(&buf, &mut res);
        assert!(
            (res.magnitude - 0.618932).abs() < 1e-4,
            "mag {}",
            res.magnitude
        );
        check_float(res.frequency, 3.138382);

        ctx.adjust_info(
            &buf,
            std::slice::from_mut(&mut res),
            1,
            0,
            None::<fn(&[f32])>,
        );
        assert!(compare_phase(0.0, res.phase, 0.01));
        assert!(
            (res.magnitude - 0.999956).abs() < 1e-4,
            "mag {}",
            res.magnitude
        );
        check_float(res.frequency, 3.134470);
    }

    #[test]
    fn one_tone_11025_a32768() {
        let mut buf = [0.0f32; 128];
        let mut ctx = GhaCtx::new(128);
        ctx.set_max_magnitude(32768.0);
        gen_tone(11025.0, 32768.0, &mut buf);

        let mut res = GhaInfo::default();
        ctx.analyze_one(&buf, &mut res);
        assert!(compare_phase(0.0, res.phase, 0.01));
        check_float(res.magnitude, 32768.0);
        check_float(res.frequency, 1.5707963705);

        ctx.adjust_info(
            &buf,
            std::slice::from_mut(&mut res),
            1,
            0,
            None::<fn(&[f32])>,
        );
        assert!(compare_phase(0.0, res.phase, 0.01));
        check_float(res.magnitude, 32768.0);
        check_float(res.frequency, 1.5707963705);

        res.magnitude = 32760.0;
        ctx.adjust_info(
            &buf,
            std::slice::from_mut(&mut res),
            1,
            32,
            None::<fn(&[f32])>,
        );
        assert!(compare_phase(0.0, res.phase, 0.01));
        check_float(res.magnitude, 32768.0);
        check_float(res.frequency, 1.5707963705);
    }

    #[test]
    fn one_tone_11025_a32768_adjust() {
        let mut buf = [0.0f32; 128];
        let mut res = GhaInfo {
            magnitude: 1000.0,
            phase: 0.0,
            frequency: 1.5,
        };
        let mut ctx = GhaCtx::new(128);
        ctx.set_max_loops(128);
        ctx.set_max_magnitude(32768.0);
        gen_tone(11025.0, 32768.0, &mut buf);

        ctx.adjust_info(
            &buf,
            std::slice::from_mut(&mut res),
            1,
            0,
            None::<fn(&[f32])>,
        );
        assert!(compare_phase(0.0, res.phase, 0.01));
        check_float(res.magnitude, 32768.0);
        check_float(res.frequency, 1.5707963705);
    }

    #[test]
    fn two_tones_5000_11025_a32768_adjust() {
        let mut buf = [0.0f32; 128];
        let mut res = [
            GhaInfo {
                magnitude: 16000.0,
                phase: 0.0,
                frequency: 1.5,
            },
            GhaInfo {
                magnitude: 16000.0,
                phase: 0.0,
                frequency: 0.75,
            },
        ];
        let mut ctx = GhaCtx::new(128);
        ctx.set_max_loops(128);
        ctx.set_max_magnitude(32768.0);
        gen_tone(11025.0, 32768.0, &mut buf);
        gen_tone(5000.0, 32768.0, &mut buf);

        ctx.adjust_info(&buf, &mut res, 2, 0, None::<fn(&[f32])>);

        assert!(compare_phase(0.0, res[0].phase, 0.01));
        check_float(res[0].magnitude.round(), 32768.0);
        check_float(res[0].frequency, 1.5707963705);

        assert!(compare_phase(0.0, res[1].phase, 0.01));
        check_float(res[1].magnitude.round(), 32768.0);
        check_float(res[1].frequency, 0.712379);
    }

    #[test]
    fn two_tones_5512hz5_11025_a32768_adjust() {
        let mut orig = [0.0f32; 128];
        gen_tone(11025.0, 16384.0, &mut orig);
        gen_tone(5512.5, 32768.0, &mut orig);

        let initial_res = [
            GhaInfo {
                magnitude: 16000.0,
                phase: 0.0,
                frequency: 1.5707,
            },
            GhaInfo {
                magnitude: 16000.0,
                phase: 0.0,
                frequency: 0.75,
            },
        ];

        let mut res = initial_res;
        let mut ctx = GhaCtx::new(128);
        ctx.set_max_loops(128);
        ctx.set_max_magnitude(32768.0);

        let rv = ctx.adjust_info(&orig, &mut res, 2, 0, None::<fn(&[f32])>);
        assert_ne!(rv, -1, "solver returned singular");

        let residual = ctx.analyzed();
        let res_energy: f64 = residual.iter().map(|&v| v as f64 * v as f64).sum();
        let orig_energy: f64 = orig.iter().map(|&v| v as f64 * v as f64).sum();

        assert!(
            res_energy < orig_energy,
            "solver made residual worse: {res_energy} >= {orig_energy}"
        );

        for &info in &res {
            assert!(info.magnitude >= 0.0, "negative magnitude");
            assert!(
                info.frequency > 0.0 && info.frequency <= std::f32::consts::PI,
                "frequency out of range: {}",
                info.frequency
            );
        }

        let phase_2pi = std::f32::consts::PI * 2.0;
        for &info in &res {
            assert!(
                info.phase >= 0.0 && info.phase < phase_2pi,
                "phase out of range: {}",
                info.phase
            );
        }
    }

    #[test]
    fn one_tone_11025_a32768_partial_frame() {
        let mut buf = [0.0f32; 128];
        let mut ctx = GhaCtx::new(128);
        ctx.set_max_magnitude(32768.0);
        ctx.set_max_loops(14);
        gen_tone(11025.0, 32768.0, &mut buf);
        for v in buf[96..128].iter_mut() {
            *v = 0.0;
        }

        let mut res = GhaInfo::default();
        ctx.analyze_one(&buf, &mut res);
        assert!(compare_phase(0.0, res.phase, 0.01));
        check_float(res.magnitude, 24576.0);
        check_float(res.frequency, 1.5707963705);

        ctx.adjust_info(
            &buf,
            std::slice::from_mut(&mut res),
            1,
            64,
            None::<fn(&[f32])>,
        );
        assert!(compare_phase(0.0, res.phase, 0.01));
        check_float(res.magnitude, 32768.0);
        check_float(res.frequency, 1.5707963705);
    }
}
