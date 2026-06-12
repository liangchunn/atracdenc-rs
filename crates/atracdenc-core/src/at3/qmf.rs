use crate::{at3::data::NUM_SAMPLES, dsp::qmf::Qmf};

pub struct Atrac3AnalysisFilterBank {
    qmf1: Qmf<NUM_SAMPLES>,
    qmf2: Qmf<{ NUM_SAMPLES / 2 }>,
    qmf3: Qmf<{ NUM_SAMPLES / 2 }>,
    buf1: Vec<f32>,
    buf2: Vec<f32>,
}

impl Atrac3AnalysisFilterBank {
    pub fn new() -> Self {
        Self {
            qmf1: Qmf::new(),
            qmf2: Qmf::new(),
            qmf3: Qmf::new(),
            buf1: vec![0.0; NUM_SAMPLES],
            buf2: vec![0.0; NUM_SAMPLES],
        }
    }

    pub fn analysis(&mut self, pcm: &[f32], subs: &mut [&mut [f32]; 4]) {
        assert_eq!(NUM_SAMPLES, pcm.len());
        for sub in subs.iter() {
            assert_eq!(NUM_SAMPLES / 4, sub.len());
        }

        self.qmf1.analysis(
            pcm,
            &mut self.buf1[..NUM_SAMPLES / 2],
            &mut self.buf2[..NUM_SAMPLES / 2],
        );

        let [sub0, sub1, sub2, sub3] = subs;
        self.qmf2
            .analysis(&self.buf1[..NUM_SAMPLES / 2], *sub0, *sub1);
        self.qmf3
            .analysis(&self.buf2[..NUM_SAMPLES / 2], *sub3, *sub2);
    }
}

impl Default for Atrac3AnalysisFilterBank {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_splits_pcm_into_four_finite_bands() {
        let mut filter_bank = Atrac3AnalysisFilterBank::new();
        let pcm = (0..NUM_SAMPLES)
            .map(|i| (2.0 * std::f32::consts::PI * 997.0 * i as f32 / 44_100.0).sin())
            .collect::<Vec<_>>();
        let mut sub0 = vec![0.0; NUM_SAMPLES / 4];
        let mut sub1 = vec![0.0; NUM_SAMPLES / 4];
        let mut sub2 = vec![0.0; NUM_SAMPLES / 4];
        let mut sub3 = vec![0.0; NUM_SAMPLES / 4];
        let mut subs = [
            sub0.as_mut_slice(),
            sub1.as_mut_slice(),
            sub2.as_mut_slice(),
            sub3.as_mut_slice(),
        ];

        filter_bank.analysis(&pcm, &mut subs);

        let all = sub0.iter().chain(&sub1).chain(&sub2).chain(&sub3);
        assert!(all.clone().all(|x| x.is_finite()));
        assert!(all.map(|x| x.abs()).sum::<f32>() > 1.0e-4);
    }
}
