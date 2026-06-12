use rustfft::num_complex::Complex;

use super::fft::FftPlan;

const TAP_HALF: [f32; 24] = [
    -0.00001461907,
    -0.00009205479,
    -0.000056157569,
    0.00030117269,
    0.0002422519,
    -0.00085293897,
    -0.0005205574,
    0.0020340169,
    0.00078333891,
    -0.0042153862,
    -0.00075614988,
    0.0078402944,
    -0.000061169922,
    -0.01344162,
    0.0024626821,
    0.021736089,
    -0.007801671,
    -0.034090221,
    0.01880949,
    0.054326009,
    -0.043596379,
    -0.099384367,
    0.13207909,
    0.46424159,
];

fn qmf_window() -> [f32; 48] {
    let mut window = [0.0; 48];
    for i in 0..24 {
        window[i] = TAP_HALF[i] * 2.0;
        window[47 - i] = TAP_HALF[i] * 2.0;
    }
    window
}

pub struct Qmf<const N_IN: usize> {
    pcm_buffer: Vec<f32>,
    pcm_buffer_merge: Vec<f32>,
}

impl<const N_IN: usize> Qmf<N_IN> {
    pub fn new() -> Self {
        assert!(N_IN >= 2 && N_IN % 4 == 0);
        Self {
            pcm_buffer: vec![0.0; N_IN + 46],
            pcm_buffer_merge: vec![0.0; N_IN + 46],
        }
    }

    pub fn analysis(&mut self, input: &[f32], lower: &mut [f32], upper: &mut [f32]) {
        assert_eq!(N_IN, input.len());
        assert_eq!(N_IN / 2, lower.len());
        assert_eq!(N_IN / 2, upper.len());

        let window = qmf_window();
        self.pcm_buffer.copy_within(N_IN..N_IN + 46, 0);
        self.pcm_buffer[46..46 + N_IN].copy_from_slice(input);

        for j in (0..N_IN).step_by(2) {
            let out_pos = j / 2;
            lower[out_pos] = 0.0;
            upper[out_pos] = 0.0;
            for i in 0..24 {
                lower[out_pos] += window[2 * i] * self.pcm_buffer[47 + j - (2 * i)];
                upper[out_pos] += window[2 * i + 1] * self.pcm_buffer[47 + j - (2 * i) - 1];
            }
            let temp = upper[out_pos];
            upper[out_pos] = lower[out_pos] - upper[out_pos];
            lower[out_pos] += temp;
        }
    }

    pub fn synthesis(&mut self, out: &mut [f32], lower: &[f32], upper: &[f32]) {
        assert_eq!(N_IN, out.len());
        assert_eq!(N_IN / 2, lower.len());
        assert_eq!(N_IN / 2, upper.len());

        let window = qmf_window();
        let new_part = &mut self.pcm_buffer_merge[46..];
        for i in (0..N_IN).step_by(4) {
            new_part[i] = lower[i / 2] + upper[i / 2];
            new_part[i + 1] = lower[i / 2] - upper[i / 2];
            new_part[i + 2] = lower[i / 2 + 1] + upper[i / 2 + 1];
            new_part[i + 3] = lower[i / 2 + 1] - upper[i / 2 + 1];
        }

        for j in 0..(N_IN / 2) {
            let win = &self.pcm_buffer_merge[j * 2..];
            let mut s1 = 0.0;
            let mut s2 = 0.0;
            for i in (0..48).step_by(2) {
                s1 += win[i] * window[i];
                s2 += win[i + 1] * window[i + 1];
            }
            out[j * 2] = s2;
            out[j * 2 + 1] = s1;
        }

        self.pcm_buffer_merge.copy_within(N_IN..N_IN + 46, 0);
    }
}

impl<const N_IN: usize> Default for Qmf<N_IN> {
    fn default() -> Self {
        Self::new()
    }
}

pub fn calc_freq_resp(sz: usize, buf: &mut [f32]) -> bool {
    let fft_sz = sz * 2;
    if fft_sz < 48 || buf.len() < sz {
        return false;
    }

    let window = qmf_window();
    let mut input = vec![Complex::default(); fft_sz];
    let start = (sz - 48) / 2;
    for (idx, sample) in window.iter().enumerate() {
        input[start + idx].re = sample / 2.0;
    }

    let mut fft = FftPlan::forward(fft_sz);
    fft.process(&mut input);

    for i in 0..sz {
        buf[i] = input[i].re * input[i].re + input[i].im * input[i].im;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_freq_resp_rejects_too_short_buffers() {
        let mut buf = [0.0; 23];
        assert!(!calc_freq_resp(23, &mut buf));
    }

    #[test]
    fn calc_freq_resp_populates_response() {
        let mut buf = [0.0; 64];
        assert!(calc_freq_resp(64, &mut buf));
        assert!(buf.iter().any(|x| *x > 0.0));
    }

    #[test]
    fn analysis_synthesis_sine_is_delayed_with_stable_gain() {
        const N: usize = 512;
        let mut analysis = Qmf::<N>::new();
        let mut synthesis = Qmf::<N>::new();
        let mut input = vec![0.0; N * 8];
        let mut output = vec![0.0; N * 8];
        let mut lower = vec![0.0; N / 2];
        let mut upper = vec![0.0; N / 2];
        let mut block_out = vec![0.0; N];

        for (i, sample) in input.iter_mut().enumerate() {
            *sample = (2.0 * std::f32::consts::PI * 997.0 * i as f32 / 44_100.0).sin();
        }

        for block in 0..8 {
            let range = block * N..(block + 1) * N;
            analysis.analysis(&input[range.clone()], &mut lower, &mut upper);
            synthesis.synthesis(&mut block_out, &lower, &upper);
            output[range].copy_from_slice(&block_out);
        }

        let mut best_delay = 0;
        let mut best_err = f32::MAX;
        let mut best_gain = 0.0;
        for delay in 0..96 {
            let mut dot = 0.0;
            let mut norm = 0.0;
            for i in N..(input.len() - delay) {
                dot += input[i] * output[i + delay];
                norm += input[i] * input[i];
            }
            let gain = dot / norm;
            let mut err = 0.0;
            let mut count = 0;
            for i in N..(input.len() - delay) {
                err += (gain * input[i] - output[i + delay]).abs();
                count += 1;
            }
            err /= count as f32;
            if err < best_err {
                best_err = err;
                best_delay = delay;
                best_gain = gain;
            }
        }

        assert_eq!(46, best_delay);
        assert!((best_gain - 2.000_329).abs() < 0.000_01, "gain {best_gain}");
        assert!(best_err < 0.000_01, "delay {best_delay}, err {best_err}");
    }
}
