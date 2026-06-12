//! Port of `atracdenc/src/lib/mdct/mdct_ut.cpp`.
//!
//! Compares the fast `Mdct`/`Midct` against a naive O(N^2) reference for
//! N = 32/64/128/256 (ramp input) plus a seeded-random N = 256 case, for both
//! the forward and inverse transforms.

use std::f64::consts::PI;

use atracdenc_core::dsp::mdct::{Mdct, Midct};
use rand::prelude::*;
use rand::rngs::StdRng;

/// Matches `CalcEps` in `mdct_ut_common.h`: magnitude * 10^(-114/20).
fn calc_eps(magn: f32) -> f32 {
    magn * 10.0_f32.powf(-114.0 / 20.0)
}

fn max_magnitude(a: &[f32], b: &[f32]) -> f32 {
    a.iter().chain(b).map(|x| x.abs()).fold(0.0_f32, f32::max)
}

/// Naive forward MDCT with N coefficients over 2N input samples,
/// mirroring the C++ reference `mdct(x, N)`.
fn reference_mdct(x: &[f32], n: usize) -> Vec<f32> {
    let mut res = Vec::with_capacity(n);
    for k in 0..n {
        let mut sum = 0.0_f32;
        for (i, sample) in x.iter().take(2 * n).enumerate() {
            sum += *sample
                * ((PI / n as f64) * (i as f64 + 0.5 + n as f64 / 2.0) * (k as f64 + 0.5)).cos()
                    as f32;
        }
        res.push(sum);
    }
    res
}

/// Naive inverse MDCT producing 2N samples from N coefficients,
/// mirroring the C++ reference `midct(x, N)`.
fn reference_midct(x: &[f32], n: usize) -> Vec<f32> {
    let mut res = Vec::with_capacity(2 * n);
    for i in 0..(2 * n) {
        let mut sum = 0.0_f32;
        for (k, sample) in x.iter().take(n).enumerate() {
            sum += *sample
                * ((PI / n as f64) * (i as f64 + 0.5 + n as f64 / 2.0) * (k as f64 + 0.5)).cos()
                    as f32;
        }
        res.push(sum);
    }
    res
}

fn fill_random(dst: &mut [f32], seed: u32) {
    let mut rng = StdRng::seed_from_u64(seed as u64);
    for x in dst {
        *x = rng.gen_range(-32768.0..32768.0);
    }
}

fn assert_near(a: &[f32], b: &[f32], eps: f32) {
    assert_eq!(a.len(), b.len());
    for (i, (l, r)) in a.iter().zip(b).enumerate() {
        assert!((l - r).abs() <= eps, "idx {i}: {l} != {r}, eps {eps}");
    }
}

// `TMDCT<N>(N)` maps to `Mdct::new(N, N)`: input length N (== 2 * coeffs),
// output length N/2.
fn run_mdct_ramp(n: usize, eps_scale: f32) {
    let mut transform = Mdct::new(n, n as f32);
    let src = (0..n).map(|i| i as f32).collect::<Vec<_>>();
    let res1 = reference_mdct(&src, n / 2);
    let res2 = transform.transform(&src);
    assert_eq!(res1.len(), res2.len());
    assert_near(&res1, res2, calc_eps(eps_scale));
}

// `TMIDCT<N>` maps to `Midct::with_default_scale(N)`: input length N/2,
// output length N.
fn run_midct_ramp(n: usize, eps_scale: f32) {
    let mut transform = Midct::with_default_scale(n);
    let src = (0..n / 2).map(|i| i as f32).collect::<Vec<_>>();
    let res1 = reference_midct(&src, n / 2);
    let res2 = transform.transform(&src);
    assert_eq!(res1.len(), res2.len());
    assert_near(&res1, res2, calc_eps(eps_scale));
}

#[test]
fn mdct_32() {
    run_mdct_ramp(32, 32.0);
}

#[test]
fn mdct_64() {
    run_mdct_ramp(64, 64.0);
}

#[test]
fn mdct_128() {
    run_mdct_ramp(128, 128.0 * 4.0);
}

#[test]
fn mdct_256() {
    run_mdct_ramp(256, 256.0 * 4.0);
}

#[test]
fn mdct_256_rand() {
    let n = 256;
    let mut transform = Mdct::new(n, n as f32);
    let mut src = vec![0.0; n];
    fill_random(&mut src, 0x4d44_3254);
    let res1 = reference_mdct(&src, n / 2);
    let res2 = transform.transform(&src);
    assert_eq!(res1.len(), res2.len());
    assert_near(&res1, res2, calc_eps(max_magnitude(&res1, res2) * 4.0));
}

#[test]
fn midct_32() {
    run_midct_ramp(32, 32.0);
}

#[test]
fn midct_64() {
    run_midct_ramp(64, 64.0);
}

#[test]
fn midct_128() {
    run_midct_ramp(128, 128.0);
}

#[test]
fn midct_256() {
    run_midct_ramp(256, 256.0 * 2.0);
}

#[test]
fn midct_256_rand() {
    let n = 256;
    let mut transform = Midct::with_default_scale(n);
    let mut src = vec![0.0; n / 2];
    fill_random(&mut src, 0x494d_4354);
    let res1 = reference_midct(&src, n / 2);
    let res2 = transform.transform(&src);
    assert_eq!(res1.len(), res2.len());
    assert_near(&res1, res2, calc_eps(max_magnitude(&res1, res2) * 4.0));
}
