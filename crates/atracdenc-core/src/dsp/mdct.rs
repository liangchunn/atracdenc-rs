use std::f64::consts::PI as PI64;

use rustfft::num_complex::Complex;

use super::fft::FftPlan;

fn calc_sin_cos(n: usize, scale: f32) -> Vec<f32> {
    let mut tmp = vec![0.0; n >> 1];
    let alpha = 2.0 * PI64 / (8.0 * n as f64);
    let omega = 2.0 * PI64 / n as f64;
    let scale = (scale as f64 / n as f64).sqrt();

    for i in 0..(n >> 2) {
        tmp[2 * i] = (scale * (omega * i as f64 + alpha).cos()) as f32;
        tmp[2 * i + 1] = (scale * (omega * i as f64 + alpha).sin()) as f32;
    }

    tmp
}

pub struct Mdct {
    n: usize,
    sincos: Vec<f32>,
    fft: FftPlan,
    fft_buf: Vec<Complex<f32>>,
    buf: Vec<f32>,
}

impl Mdct {
    pub fn new(n: usize, scale: f32) -> Self {
        assert!(n >= 4 && n % 4 == 0);
        Self {
            n,
            sincos: calc_sin_cos(n, scale),
            fft: FftPlan::forward(n >> 2),
            fft_buf: vec![Complex::default(); n >> 2],
            buf: vec![0.0; n >> 1],
        }
    }

    pub fn transform(&mut self, input: &[f32]) -> &[f32] {
        assert_eq!(self.n, input.len());

        let n2 = self.n >> 1;
        let n4 = self.n >> 2;
        let n34 = 3 * n4;
        let n54 = 5 * n4;

        let mut n = 0;
        while n < n4 {
            let r0 = input[n34 - 1 - n] + input[n34 + n];
            let i0 = input[n4 + n] - input[n4 - 1 - n];
            let c = self.sincos[n];
            let s = self.sincos[n + 1];

            self.fft_buf[n / 2].re = r0 * c + i0 * s;
            self.fft_buf[n / 2].im = i0 * c - r0 * s;
            n += 2;
        }

        while n < n2 {
            let r0 = input[n34 - 1 - n] - input[n - n4];
            let i0 = input[n4 + n] + input[n54 - 1 - n];
            let c = self.sincos[n];
            let s = self.sincos[n + 1];

            self.fft_buf[n / 2].re = r0 * c + i0 * s;
            self.fft_buf[n / 2].im = i0 * c - r0 * s;
            n += 2;
        }

        self.fft.process(&mut self.fft_buf);

        for n in (0..n2).step_by(2) {
            let r0 = self.fft_buf[n / 2].re;
            let i0 = self.fft_buf[n / 2].im;
            let c = self.sincos[n];
            let s = self.sincos[n + 1];

            self.buf[n] = -r0 * c - i0 * s;
            self.buf[n2 - 1 - n] = -r0 * s + i0 * c;
        }

        &self.buf
    }
}

impl Default for Mdct {
    fn default() -> Self {
        Self::new(256, 1.0)
    }
}

pub struct Midct {
    n: usize,
    sincos: Vec<f32>,
    fft: FftPlan,
    fft_buf: Vec<Complex<f32>>,
    buf: Vec<f32>,
}

impl Midct {
    pub fn new(n: usize, scale: f32) -> Self {
        assert!(n >= 4 && n % 4 == 0);
        Self {
            n,
            sincos: calc_sin_cos(n, scale / 2.0),
            fft: FftPlan::forward(n >> 2),
            fft_buf: vec![Complex::default(); n >> 2],
            buf: vec![0.0; n],
        }
    }

    pub fn with_default_scale(n: usize) -> Self {
        Self::new(n, n as f32)
    }

    pub fn transform(&mut self, input: &[f32]) -> &[f32] {
        assert_eq!(self.n >> 1, input.len());

        let n2 = self.n >> 1;
        let n4 = self.n >> 2;
        let n34 = 3 * n4;
        let n54 = 5 * n4;

        for n in (0..n2).step_by(2) {
            let r0 = input[n];
            let i0 = input[n2 - 1 - n];
            let c = self.sincos[n];
            let s = self.sincos[n + 1];

            self.fft_buf[n / 2].re = -2.0 * (i0 * s + r0 * c);
            self.fft_buf[n / 2].im = -2.0 * (i0 * c - r0 * s);
        }

        self.fft.process(&mut self.fft_buf);

        let mut n = 0;
        while n < n4 {
            let r0 = self.fft_buf[n / 2].re;
            let i0 = self.fft_buf[n / 2].im;
            let c = self.sincos[n];
            let s = self.sincos[n + 1];

            let r1 = r0 * c + i0 * s;
            let i1 = r0 * s - i0 * c;

            self.buf[n34 - 1 - n] = r1;
            self.buf[n34 + n] = r1;
            self.buf[n4 + n] = i1;
            self.buf[n4 - 1 - n] = -i1;
            n += 2;
        }

        while n < n2 {
            let r0 = self.fft_buf[n / 2].re;
            let i0 = self.fft_buf[n / 2].im;
            let c = self.sincos[n];
            let s = self.sincos[n + 1];

            let r1 = r0 * c + i0 * s;
            let i1 = r0 * s - i0 * c;

            self.buf[n34 - 1 - n] = r1;
            self.buf[n - n4] = -r1;
            self.buf[n4 + n] = i1;
            self.buf[n54 - 1 - n] = i1;
            n += 2;
        }

        &self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill_random(dst: &mut [f32], seed: u32) {
        let mut state = seed;
        for x in dst {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let unit = (state as f32) / (u32::MAX as f32);
            *x = -32768.0 + unit * 65535.0;
        }
    }

    fn max_magnitude(a: &[f32], b: &[f32]) -> f32 {
        a.iter().chain(b).map(|x| x.abs()).fold(0.0_f32, f32::max)
    }

    fn calc_eps(magn: f32) -> f32 {
        magn * 10.0_f32.powf(-114.0 / 20.0)
    }

    fn reference_mdct(x: &[f32], n: usize) -> Vec<f32> {
        let mut res = Vec::with_capacity(n);
        for k in 0..n {
            let mut sum = 0.0;
            for (i, sample) in x.iter().take(2 * n).enumerate() {
                let term = (*sample as f64)
                    * ((PI64 / n as f64) * (i as f64 + 0.5 + n as f64 / 2.0) * (k as f64 + 0.5))
                        .cos();
                sum = (sum as f64 + term) as f32;
            }
            res.push(sum);
        }
        res
    }

    fn reference_midct(x: &[f32], n: usize) -> Vec<f32> {
        let mut res = Vec::with_capacity(2 * n);
        for i in 0..(2 * n) {
            let mut sum = 0.0;
            for (k, sample) in x.iter().take(n).enumerate() {
                let term = (*sample as f64)
                    * ((PI64 / n as f64) * (i as f64 + 0.5 + n as f64 / 2.0) * (k as f64 + 0.5))
                        .cos();
                sum = (sum as f64 + term) as f32;
            }
            res.push(sum);
        }
        res
    }

    fn assert_close(a: &[f32], b: &[f32], eps: f32) {
        assert_eq!(a.len(), b.len());
        for (idx, (left, right)) in a.iter().zip(b).enumerate() {
            let tolerance = eps.max(1.0e-4).max(left.abs().max(right.abs()) * 1.0e-6);
            assert!(
                (left - right).abs() <= tolerance,
                "idx {idx}: {left} != {right}, tolerance {tolerance}"
            );
        }
    }

    #[test]
    fn mdct_reference_vectors() {
        for n in [32_usize, 64, 128, 256, 512] {
            let mut transform = Mdct::new(n, n as f32);
            let src = (0..n).map(|i| i as f32).collect::<Vec<_>>();
            let res1 = reference_mdct(&src, n / 2);
            let res2 = transform.transform(&src);
            let eps_scale = if n >= 128 { n * 4 } else { n };
            assert_close(&res1, res2, calc_eps(eps_scale as f32));
        }
    }

    #[test]
    fn mdct_random_256() {
        let n = 256;
        let mut transform = Mdct::new(n, n as f32);
        let mut src = vec![0.0; n];
        fill_random(&mut src, 0x4d44_3254);
        let res1 = reference_mdct(&src, n / 2);
        let res2 = transform.transform(&src);
        assert_close(&res1, res2, calc_eps(max_magnitude(&res1, res2) * 4.0));
    }

    #[test]
    fn midct_reference_vectors() {
        for n in [32_usize, 64, 128, 256, 512] {
            let mut transform = Midct::with_default_scale(n);
            let mut src = vec![0.0; n / 2];
            for (idx, x) in src.iter_mut().enumerate() {
                *x = idx as f32;
            }
            let res1 = reference_midct(&src, n / 2);
            let res2 = transform.transform(&src);
            let eps_scale = if n == 256 { n * 2 } else { n };
            assert_close(&res1, res2, calc_eps(eps_scale as f32));
        }
    }

    #[test]
    fn midct_random_256() {
        let n = 256;
        let mut transform = Midct::with_default_scale(n);
        let mut src = vec![0.0; n / 2];
        fill_random(&mut src, 0x494d_4354);
        let res1 = reference_midct(&src, n / 2);
        let res2 = transform.transform(&src);
        assert_close(&res1, res2, calc_eps(max_magnitude(&res1, res2) * 4.0));
    }
}
