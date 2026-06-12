use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex};

pub struct FftPlan {
    fft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl FftPlan {
    pub fn forward(n: usize) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(n);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        Self { fft, scratch }
    }

    pub fn inverse(n: usize) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_inverse(n);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        Self { fft, scratch }
    }

    pub fn process(&mut self, buf: &mut [Complex<f32>]) {
        self.fft.process_with_scratch(buf, &mut self.scratch);
    }
}
