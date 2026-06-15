//! Small spectral and bit-twiddling helpers shared across modules.
//!
//! Spectrum manipulation ([`invert_spectr`], [`inverted_spectr`]) and bit
//! utilities ([`get_first_set_bit`]).

pub fn invert_spectr(s: &mut [f32]) {
    for sample in s.iter_mut().step_by(2) {
        *sample *= -1.0;
    }
}

pub fn inverted_spectr(s: &[f32]) -> Vec<f32> {
    let mut out = s.to_vec();
    invert_spectr(&mut out);
    out
}

pub fn get_first_set_bit(x: u32) -> u16 {
    if x == 0 {
        return 0;
    }
    (31 - x.leading_zeros()) as u16
}

pub fn div8_ceil(x: u32) -> u32 {
    assert!(x > 0);
    1 + (x - 1) / 8
}

pub fn calc_median(input: &[f32]) -> f32 {
    assert!(!input.is_empty());
    let mut tmp = input.to_vec();
    tmp.sort_by(|a, b| a.total_cmp(b));
    tmp[(tmp.len() - 1) / 2]
}

pub fn to_int(x: f32) -> i32 {
    x.round_ties_even() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_first_set_bit_test() {
        assert_eq!(1, get_first_set_bit(2));
        assert_eq!(1, get_first_set_bit(3));
        assert_eq!(2, get_first_set_bit(4));
        assert_eq!(2, get_first_set_bit(5));
        assert_eq!(2, get_first_set_bit(6));
        assert_eq!(2, get_first_set_bit(7));
        assert_eq!(3, get_first_set_bit(8));
        assert_eq!(3, get_first_set_bit(9));
        assert_eq!(3, get_first_set_bit(10));
        assert_eq!(0, get_first_set_bit(0));
    }

    #[test]
    fn extras_from_plan() {
        let mut spectra = [1.0, 2.0, -3.0, -4.0];
        invert_spectr(&mut spectra);
        assert_eq!([-1.0, 2.0, 3.0, -4.0], spectra);
        assert_eq!(1, div8_ceil(1));
        assert_eq!(1, div8_ceil(8));
        assert_eq!(2, div8_ceil(9));
        assert_eq!(2.0, calc_median(&[3.0, 1.0, 2.0]));
        assert_eq!(2, to_int(1.5));
        assert_eq!(2, to_int(2.5));
    }
}
