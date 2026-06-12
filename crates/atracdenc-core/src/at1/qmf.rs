use crate::dsp::qmf::Qmf;

const N_IN_SAMPLES: usize = 512;
const DELAY_COMP: usize = 39;

pub struct Atrac1AnalysisFilterBank {
    qmf1: Qmf<N_IN_SAMPLES>,
    qmf2: Qmf<{ N_IN_SAMPLES / 2 }>,
    mid_low_tmp: Vec<f32>,
    delay_buf: Vec<f32>,
}

impl Atrac1AnalysisFilterBank {
    pub fn new() -> Self {
        Self {
            qmf1: Qmf::new(),
            qmf2: Qmf::new(),
            mid_low_tmp: vec![0.0; 512],
            delay_buf: vec![0.0; DELAY_COMP + 512],
        }
    }

    pub fn analysis(&mut self, pcm: &[f32], low: &mut [f32], mid: &mut [f32], hi: &mut [f32]) {
        assert_eq!(N_IN_SAMPLES, pcm.len());
        assert_eq!(128, low.len());
        assert_eq!(128, mid.len());
        assert_eq!(256, hi.len());

        self.delay_buf.copy_within(256..256 + DELAY_COMP, 0);
        self.qmf1.analysis(
            pcm,
            &mut self.mid_low_tmp[..256],
            &mut self.delay_buf[DELAY_COMP..DELAY_COMP + 256],
        );
        self.qmf2.analysis(&self.mid_low_tmp[..256], low, mid);
        hi.copy_from_slice(&self.delay_buf[..256]);
    }
}

impl Default for Atrac1AnalysisFilterBank {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Atrac1SynthesisFilterBank {
    qmf1: Qmf<N_IN_SAMPLES>,
    qmf2: Qmf<{ N_IN_SAMPLES / 2 }>,
    mid_low_tmp: Vec<f32>,
    delay_buf: Vec<f32>,
}

impl Atrac1SynthesisFilterBank {
    pub fn new() -> Self {
        Self {
            qmf1: Qmf::new(),
            qmf2: Qmf::new(),
            mid_low_tmp: vec![0.0; 512],
            delay_buf: vec![0.0; DELAY_COMP + 512],
        }
    }

    pub fn synthesis(&mut self, pcm: &mut [f32], low: &[f32], mid: &[f32], hi: &[f32]) {
        assert_eq!(N_IN_SAMPLES, pcm.len());
        assert_eq!(128, low.len());
        assert_eq!(128, mid.len());
        assert_eq!(256, hi.len());

        self.delay_buf.copy_within(256..256 + DELAY_COMP, 0);
        self.delay_buf[DELAY_COMP..DELAY_COMP + 256].copy_from_slice(hi);
        self.qmf2.synthesis(&mut self.mid_low_tmp[..256], low, mid);
        self.qmf1
            .synthesis(pcm, &self.mid_low_tmp[..256], &self.delay_buf[..256]);
    }
}

impl Default for Atrac1SynthesisFilterBank {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_synthesis_filterbank_is_stable() {
        let mut analysis = Atrac1AnalysisFilterBank::new();
        let mut synthesis = Atrac1SynthesisFilterBank::new();
        let mut low = vec![0.0; 128];
        let mut mid = vec![0.0; 128];
        let mut hi = vec![0.0; 256];
        let mut pcm_out = vec![0.0; 512];
        let pcm = (0..512)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44_100.0).sin())
            .collect::<Vec<_>>();

        analysis.analysis(&pcm, &mut low, &mut mid, &mut hi);
        synthesis.synthesis(&mut pcm_out, &low, &mid, &hi);

        assert!(low.iter().chain(&mid).chain(&hi).any(|x| x.abs() > 1.0e-5));
        assert!(pcm_out.iter().all(|x| x.is_finite()));
    }
}
