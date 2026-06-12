use super::mdct::Midct;

pub struct Dct4 {
    n: usize,
    midct: Midct,
    buf: Vec<f32>,
}

impl Dct4 {
    pub fn new(n: usize, scale: f32) -> Self {
        assert!(n > 0 && n % 2 == 0);
        Self {
            n,
            midct: Midct::new(n * 2, (n * 2) as f32 * scale),
            buf: vec![0.0; n],
        }
    }

    pub fn transform(&mut self, input: &[f32]) -> &[f32] {
        assert_eq!(self.n, input.len());
        let x = self.midct.transform(input);
        for i in 0..self.n {
            self.buf[i] = -x[i + self.n / 2];
        }
        &self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dct4_16_matches_midct_adapter_contract() {
        let mut dct = Dct4::new(16, 1.0);
        let mut midct = Midct::new(32, 32.0);
        let input = (0..16).map(|i| i as f32 - 8.0).collect::<Vec<_>>();
        let expected_midct = midct.transform(&input);
        let expected = (0..16).map(|i| -expected_midct[i + 8]).collect::<Vec<_>>();
        let got = dct.transform(&input);

        for (expected, got) in expected.iter().zip(got) {
            assert!((expected - got).abs() < 1.0e-4);
        }
    }
}
