use std::f32::consts::PI;

use rustfft::num_complex::Complex;

use super::fft::FftPlan;

#[derive(Debug, Clone)]
pub struct ProcessResult {
    pub signal: Vec<f32>,
    pub high_freq_ratio: f32,
}

pub struct SpectralUpsampler {
    low_cut_bin: usize,
    win: Vec<f32>,
    fwd: FftPlan,
    inv: FftPlan,
}

impl SpectralUpsampler {
    pub const IN_N: usize = 512;
    pub const UPSAMPLE: usize = 8;
    pub const OUT_N: usize = Self::IN_N * Self::UPSAMPLE;
    pub const DEFAULT_EPS: f32 = 0.15;
    pub const HIGH_FREQ_THRESHOLD: f32 = 0.05;

    pub fn new(sample_rate: f32, low_cut_hz: f32, epsilon: f32) -> Self {
        let low_cut_bin = (low_cut_hz * Self::IN_N as f32 / sample_rate).ceil() as usize;
        let e_n = epsilon * Self::IN_N as f32;
        let f_n = Self::IN_N as f32;
        let mut win = vec![0.0; Self::IN_N];

        for (n, w) in win.iter_mut().enumerate() {
            let f = n as f32;
            *w = if n == 0 {
                0.0
            } else if f < e_n {
                let z_p = e_n * (1.0 / f + 1.0 / (f - e_n));
                1.0 / (1.0 + z_p.exp())
            } else if f <= f_n - e_n {
                1.0
            } else {
                let m = f_n - f;
                let z_p = e_n * (1.0 / m + 1.0 / (m - e_n));
                1.0 / (1.0 + z_p.exp())
            };
        }

        Self {
            low_cut_bin,
            win,
            fwd: FftPlan::forward(Self::IN_N),
            inv: FftPlan::inverse(Self::OUT_N),
        }
    }

    pub fn with_default_eps(sample_rate: f32, low_cut_hz: f32) -> Self {
        Self::new(sample_rate, low_cut_hz, Self::DEFAULT_EPS)
    }

    pub fn process(&mut self, input: &[f32]) -> ProcessResult {
        assert_eq!(Self::IN_N, input.len());

        let mut fwd = vec![Complex::default(); Self::IN_N];
        for ((dst, sample), win) in fwd.iter_mut().zip(input).zip(&self.win) {
            dst.re = sample * win;
        }
        self.fwd.process(&mut fwd);

        let mut total_e = 0.0_f64;
        let mut filt_high_e = 0.0_f64;
        for (k, bin) in fwd.iter().enumerate().take(Self::IN_N / 2 + 1) {
            let e = f64::from(bin.re) * f64::from(bin.re) + f64::from(bin.im) * f64::from(bin.im);
            total_e += e;
            let h = self.high_pass_weight(k);
            filt_high_e += e * f64::from(h * h);
        }
        let high_freq_ratio = if total_e > 0.0 {
            (filt_high_e / total_e) as f32
        } else {
            0.0
        };

        let mut inv = vec![Complex::default(); Self::OUT_N];
        let scale = Self::UPSAMPLE as f32;

        let passband_start = if self.low_cut_bin == 0 {
            0
        } else {
            self.low_cut_bin + 2
        };
        for k in passband_start..Self::IN_N / 2 {
            inv[k] = fwd[k] * scale;
        }

        if self.low_cut_bin > 0 {
            for i in 1..3 {
                let k = self.low_cut_bin - 1 + i;
                if k >= Self::IN_N / 2 {
                    continue;
                }
                let w = 0.5 * (1.0 - (PI * i as f32 / 2.0).cos());
                inv[k] = fwd[k] * scale * w;
            }
        }

        if self.low_cut_bin + 2 <= Self::IN_N / 2 {
            inv[Self::IN_N / 2].re = fwd[Self::IN_N / 2].re * scale * 0.5;
        }

        for k in 1..Self::OUT_N / 2 {
            inv[Self::OUT_N - k] = inv[k].conj();
        }

        self.inv.process(&mut inv);
        let norm = 1.0 / Self::OUT_N as f32;
        let signal = inv.into_iter().map(|x| x.re * norm).collect();

        ProcessResult {
            signal,
            high_freq_ratio,
        }
    }

    fn high_pass_weight(&self, k: usize) -> f32 {
        if self.low_cut_bin == 0 || k >= self.low_cut_bin + 2 {
            1.0
        } else if k >= self.low_cut_bin {
            let i = k - self.low_cut_bin + 1;
            0.5 * (1.0 - (PI * i as f32 / 2.0).cos())
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::transient::{CurveBuilderCtx, analyze_gain, calc_curve};

    const SAMPLE_RATE: f32 = 11_024.0;

    fn rms(data: &[f32]) -> f32 {
        (data.iter().map(|x| x * x).sum::<f32>() / data.len() as f32).sqrt()
    }

    fn fill_sine(buf: &mut [f32], freq_hz: f32, sample_rate: f32) {
        for (i, sample) in buf.iter_mut().enumerate() {
            let phase =
                2.0 * std::f64::consts::PI * f64::from(freq_hz) * i as f64 / f64::from(sample_rate);
            *sample = phase.sin() as f32;
        }
    }

    fn planck_windowed_rms(input: &[f32], eps: f32) -> f32 {
        let e_n = eps * SpectralUpsampler::IN_N as f32;
        let f_n = SpectralUpsampler::IN_N as f32;
        let mut acc = 0.0_f64;
        for (i, sample) in input.iter().enumerate().take(384).skip(128) {
            let f = i as f32;
            let w = if i == 0 {
                0.0
            } else if f < e_n {
                let z_p = e_n * (1.0 / f + 1.0 / (f - e_n));
                1.0 / (1.0 + z_p.exp())
            } else if f <= f_n - e_n {
                1.0
            } else {
                let m = f_n - f;
                let z_p = e_n * (1.0 / m + 1.0 / (m - e_n));
                1.0 / (1.0 + z_p.exp())
            };
            let v = f64::from(*sample * w);
            acc += v * v;
        }
        (acc / 256.0).sqrt() as f32
    }

    #[test]
    fn output_size() {
        let mut proc = SpectralUpsampler::with_default_eps(SAMPLE_RATE, 500.0);
        let input = vec![1.0; SpectralUpsampler::IN_N];
        let result = proc.process(&input);
        assert_eq!(SpectralUpsampler::OUT_N, result.signal.len());
    }

    #[test]
    fn dc_is_removed_by_low_cut_filter() {
        let mut proc = SpectralUpsampler::with_default_eps(SAMPLE_RATE, 500.0);
        let input = vec![1.0; SpectralUpsampler::IN_N];
        let result = proc.process(&input);
        assert!(rms(&result.signal[1024..3072]) < 0.01);
    }

    #[test]
    fn high_frequency_sine_preserves_rms() {
        for freq_hz in [1378.0, 2756.0, 4134.0, 2000.0, 3000.0] {
            let mut proc = SpectralUpsampler::with_default_eps(SAMPLE_RATE, 500.0);
            let mut input = vec![0.0; SpectralUpsampler::IN_N];
            fill_sine(&mut input, freq_hz, SAMPLE_RATE);
            let result = proc.process(&input);
            let ref_rms = planck_windowed_rms(&input, SpectralUpsampler::DEFAULT_EPS);
            let out_rms = rms(&result.signal[1024..3072]);
            assert!(
                (out_rms - ref_rms).abs() <= 0.05 * ref_rms,
                "freq {freq_hz}: out {out_rms}, ref {ref_rms}"
            );
        }
    }

    #[test]
    fn chirp_no_false_transient_short_regression() {
        let signal_len = 16_384;
        let fs = 11_025.0_f32;
        let low_cut_hz = 689.0_f32;
        let t = signal_len as f64 / f64::from(fs);
        let mut signal = vec![0.0; signal_len];
        for (i, sample) in signal.iter_mut().enumerate() {
            let time = i as f64 / f64::from(fs);
            let phase = 2.0 * std::f64::consts::PI * (0.5 * 5510.0 * time * time / t);
            *sample = phase.sin() as f32;
        }

        let num_frames = (signal_len as isize - 384) / 256 + 1;
        let mut upsampler = SpectralUpsampler::with_default_eps(fs, low_cut_hz);
        let mut ctx = CurveBuilderCtx::default();

        for frame in 0..num_frames {
            let base = frame as usize * 256;
            let mut up_input = vec![0.0; SpectralUpsampler::IN_N];
            for (j, up_val) in up_input.iter_mut().enumerate().take(128) {
                let idx = base as isize - 128 + j as isize;
                if idx >= 0 {
                    *up_val = signal[idx as usize];
                }
            }
            up_input[128..384].copy_from_slice(&signal[base..base + 256]);
            up_input[384..512].copy_from_slice(&signal[base + 256..base + 384]);

            let result = upsampler.process(&up_input);
            if result.high_freq_ratio < SpectralUpsampler::HIGH_FREQ_THRESHOLD {
                ctx.last_level = 0.0;
                continue;
            }

            let gain = analyze_gain(&result.signal[1024..3072], 32, true, None, None);
            let curve = calc_curve(&gain, &mut ctx, None, 2.0, None, None, None);
            assert!(curve.is_empty(), "frame {frame}: {curve:?}");
        }
    }
}
