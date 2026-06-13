use rustfft::num_complex::Complex;

use super::fft::FftPlan;

const TAP_HALF: [f32; 24] = [
    -0.00001461907,
    -0.00009205479,
    -0.000_056_157_57,
    0.000_301_172_7,
    0.0002422519,
    -0.000_852_939,
    -0.0005205574,
    0.002_034_017,
    0.000_783_338_9,
    -0.004_215_386,
    -0.000_756_149_9,
    0.007_840_294,
    -0.000_061_169_92,
    -0.01344162,
    0.002_462_682,
    0.021_736_09,
    -0.007801671,
    -0.034_090_22,
    0.01880949,
    0.054_326_01,
    -0.043_596_38,
    -0.099_384_37,
    0.132_079_1,
    0.464_241_6,
];

const QMF_WINDOW: [f32; 48] = {
    let mut window = [0.0_f32; 48];
    let mut i = 0;
    while i < 24 {
        window[i] = TAP_HALF[i] * 2.0;
        window[47 - i] = TAP_HALF[i] * 2.0;
        i += 1;
    }
    window
};

const QMF_WINDOW_EVEN: [f32; 24] = {
    let mut arr = [0.0_f32; 24];
    let mut i = 0;
    while i < 24 {
        arr[i] = QMF_WINDOW[2 * i];
        i += 1;
    }
    arr
};

const QMF_WINDOW_ODD: [f32; 24] = {
    let mut arr = [0.0_f32; 24];
    let mut i = 0;
    while i < 24 {
        arr[i] = QMF_WINDOW[2 * i + 1];
        i += 1;
    }
    arr
};

const HALF_HISTORY: usize = 23;

#[inline]
fn qmf_window() -> [f32; 48] {
    QMF_WINDOW
}

pub struct Qmf<const N_IN: usize> {
    pcm_even: Vec<f32>,
    pcm_odd: Vec<f32>,
    pcm_sums: Vec<f32>,
    pcm_diffs: Vec<f32>,
}

fn buffer_len(n_in: usize) -> usize {
    n_in / 2 + HALF_HISTORY
}

impl<const N_IN: usize> Qmf<N_IN> {
    pub fn new() -> Self {
        assert!(N_IN >= 2 && N_IN.is_multiple_of(4));
        let len = buffer_len(N_IN);
        Self {
            pcm_even: vec![0.0; len],
            pcm_odd: vec![0.0; len],
            pcm_sums: vec![0.0; len],
            pcm_diffs: vec![0.0; len],
        }
    }

    pub fn analysis(&mut self, input: &[f32], lower: &mut [f32], upper: &mut [f32]) {
        assert_eq!(N_IN, input.len());
        assert_eq!(N_IN / 2, lower.len());
        assert_eq!(N_IN / 2, upper.len());

        let half = N_IN / 2;
        self.pcm_even.copy_within(half..half + HALF_HISTORY, 0);
        self.pcm_odd.copy_within(half..half + HALF_HISTORY, 0);
        for k in 0..half {
            self.pcm_even[HALF_HISTORY + k] = input[2 * k];
            self.pcm_odd[HALF_HISTORY + k] = input[2 * k + 1];
        }

        for out_pos in 0..half {
            let off = out_pos;
            let lo = QMF_WINDOW_ODD
                .iter()
                .zip(&self.pcm_odd[off..off + 24])
                .map(|(w, p)| w * p)
                .sum::<f32>();
            let hi = QMF_WINDOW_EVEN
                .iter()
                .zip(&self.pcm_even[off..off + 24])
                .map(|(w, p)| w * p)
                .sum::<f32>();
            let temp = hi;
            upper[out_pos] = lo - hi;
            lower[out_pos] = lo + temp;
        }
    }

    pub fn synthesis(&mut self, out: &mut [f32], lower: &[f32], upper: &[f32]) {
        assert_eq!(N_IN, out.len());
        assert_eq!(N_IN / 2, lower.len());
        assert_eq!(N_IN / 2, upper.len());

        let half = N_IN / 2;
        for j in 0..half {
            self.pcm_sums[HALF_HISTORY + j] = lower[j] + upper[j];
            self.pcm_diffs[HALF_HISTORY + j] = lower[j] - upper[j];
        }

        for j in 0..half {
            let off = j;
            let s1 = QMF_WINDOW_EVEN
                .iter()
                .zip(&self.pcm_sums[off..off + 24])
                .map(|(w, p)| w * p)
                .sum::<f32>();
            let s2 = QMF_WINDOW_ODD
                .iter()
                .zip(&self.pcm_diffs[off..off + 24])
                .map(|(w, p)| w * p)
                .sum::<f32>();
            out[j * 2] = s2;
            out[j * 2 + 1] = s1;
        }

        self.pcm_sums.copy_within(half..half + HALF_HISTORY, 0);
        self.pcm_diffs.copy_within(half..half + HALF_HISTORY, 0);
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
