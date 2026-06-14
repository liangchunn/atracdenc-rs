//! Port of `atracdenc/src/gain_processor_ut.cpp`.
//!
//! Five groups:
//!   * `energy_scale_*`   — `Atrac3Mdct::calc_gain_energy_scale` behaviour.
//!   * `modulate_*`       — per-sample attenuation rules of `modulate`.
#![allow(clippy::needless_range_loop)]

//!   * `demodulate_*`     — per-sample re-amplification rules of `demodulate`.
//!   * `mirror_*`         — algebraic modulate -> demodulate identities.
//!   * `freq_domain_*`    — full MDCT-domain energy-reduction + roundtrip.
//!   * `boundary_level_*` — regression scenarios around the gain boundary.

use atracdenc_core::{
    at3::{
        data::{ENCODE_WINDOW, EXPONENT_OFFSET, NUM_QMF, NUM_SAMPLES},
        mdct::{At3GainProcessor, Atrac3Mdct},
    },
    dsp::{
        gain::GainPoint,
        transient::{CurveBuilderCtx, analyze_gain, calc_curve},
        upsampler::SpectralUpsampler,
    },
};

const HALF: usize = 256;
const BAND_SZ: usize = 512;

fn gp(level: u32, location: u32) -> GainPoint {
    GainPoint { level, location }
}

/// `GainLevel[L] = 2^(ExponentOffset - L) = 2^(4 - L)`.
fn gain_level_at(l: i32) -> f32 {
    2.0_f32.powi(EXPONENT_OFFSET - l)
}

fn sqr(x: f32) -> f32 {
    x * x
}

// ============================================================================
// Gain energy scale tests
// ============================================================================

#[test]
fn energy_scale_empty_gain_is_unity() {
    let mut prev = vec![0.0_f32; 256];
    let mut cur = vec![0.0_f32; 256];
    for i in 0..256 {
        prev[i] = 0.001 * ((i % 17) + 1) as f32;
        cur[i] = 0.002 * ((i % 11) + 1) as f32;
    }

    let res = Atrac3Mdct::calc_gain_energy_scale(&prev, &cur, &[], 1.0);
    assert!((res.scale.prev_half - 1.0).abs() < 1.0e-6);
    assert!((res.scale.cur_half - 1.0).abs() < 1.0e-6);
    assert!((res.scale.frame - 1.0).abs() < 1.0e-6);
    assert!((res.next_overlap_scale - 1.0).abs() < 1.0e-6);
}

#[test]
fn energy_scale_previous_half_includes_stored_overlap_scale_and_first_gain() {
    let mut prev = vec![0.0_f32; 256];
    let cur = vec![0.0_f32; 256];
    prev[17] = 0.25;
    prev[190] = -0.5;

    let gain = [gp(2, 31)];
    let prev_overlap_scale = 1.5_f32;
    let gain_scale = gain_level_at(2);
    let res = Atrac3Mdct::calc_gain_energy_scale(&prev, &cur, &gain, prev_overlap_scale);

    let expected = prev_overlap_scale * gain_scale * gain_scale;
    assert!((res.scale.prev_half - expected).abs() < 1.0e-5);
    assert!((res.scale.frame - expected).abs() < 1.0e-5);
    assert!((res.scale.cur_half - 1.0).abs() < 1.0e-6);
    assert!((res.next_overlap_scale - 1.0).abs() < 1.0e-6);
}

#[test]
fn energy_scale_current_half_constant_region_uses_gain_squared() {
    let prev = vec![0.0_f32; 256];
    let mut cur = vec![0.0_f32; 256];
    cur[128] = 2.0;

    let gain = [gp(2, 31)];
    let gain_scale = gain_level_at(2);
    let res = Atrac3Mdct::calc_gain_energy_scale(&prev, &cur, &gain, 1.0);

    let expected = gain_scale * gain_scale;
    assert!((res.scale.prev_half - 1.0).abs() < 1.0e-6);
    assert!((res.scale.cur_half - expected).abs() < 1.0e-5);
    assert!((res.scale.frame - expected).abs() < 1.0e-5);
    assert!((res.next_overlap_scale - expected).abs() < 1.0e-5);
}

#[test]
fn energy_scale_current_and_next_overlap_use_opposite_mdct_windows() {
    let prev = vec![0.0_f32; 256];
    let mut cur = vec![0.0_f32; 256];
    cur[4] = 1.0;
    cur[240] = 1.0;

    let gain = [gp(2, 1)];
    let div = gain_level_at(2);
    let res = Atrac3Mdct::calc_gain_energy_scale(&prev, &cur, &gain, 1.0);

    let cur_w0 = ENCODE_WINDOW[255 - 4];
    let cur_w1 = ENCODE_WINDOW[255 - 240];
    let next_w0 = ENCODE_WINDOW[4];
    let next_w1 = ENCODE_WINDOW[240];
    let expected_cur = (sqr(cur_w0) + sqr(cur_w1)) / (sqr(cur_w0 / div) + sqr(cur_w1));
    let expected_next = (sqr(next_w0) + sqr(next_w1)) / (sqr(next_w0 / div) + sqr(next_w1));

    assert!((res.scale.cur_half - expected_cur).abs() < 1.0e-5);
    assert!((res.next_overlap_scale - expected_next).abs() < 1.0e-5);
    assert!(res.scale.cur_half > res.next_overlap_scale);
}

// ============================================================================
// Modulate tests
// ============================================================================

#[test]
fn modulate_empty_gain_is_noop() {
    let mut buf_cur = vec![2.0_f32; 256];
    let mut buf_next = vec![3.0_f32; 256];
    At3GainProcessor::modulate(&[], &mut buf_cur, &mut buf_next);
    assert!(buf_cur.iter().all(|x| *x == 2.0));
    assert!(buf_next.iter().all(|x| *x == 3.0));
}

#[test]
fn modulate_buf_cur_all_positions_divided_by_scale() {
    let gi = [gp(2, 31)];
    let input = 8.0_f32;
    let mut buf_cur = vec![input; 256];
    let mut buf_next = vec![1.0_f32; 256];
    At3GainProcessor::modulate(&gi, &mut buf_cur, &mut buf_next);

    let scale = gain_level_at(2);
    for (i, v) in buf_cur.iter().enumerate() {
        assert!((v - input / scale).abs() < 1.0e-6, "buf_cur at {i}");
    }
}

#[test]
fn modulate_buf_next_constant_region_divided_by_level() {
    let gi = [gp(2, 31)];
    let input = 8.0_f32;
    let mut buf_cur = vec![1.0_f32; 256];
    let mut buf_next = vec![input; 256];
    At3GainProcessor::modulate(&gi, &mut buf_cur, &mut buf_next);

    let level = gain_level_at(2);
    for (i, v) in buf_next.iter().enumerate().take(248) {
        assert!((v - input / level).abs() < 1.0e-6, "buf_next at {i}");
    }
}

#[test]
fn modulate_buf_next_remainder_unchanged() {
    let gi = [gp(2, 4)];
    let sentinel = 7.77_f32;
    let mut buf_cur = vec![1.0_f32; 256];
    let mut buf_next = vec![sentinel; 256];
    At3GainProcessor::modulate(&gi, &mut buf_cur, &mut buf_next);

    for (i, v) in buf_next.iter().enumerate().skip(40) {
        assert!((v - sentinel).abs() < 1.0e-6, "buf_next at {i}");
    }
}

#[test]
fn modulate_buf_cur_remainder_still_divided_by_scale() {
    let gi = [gp(2, 4)];
    let input = 12.0_f32;
    let mut buf_cur = vec![input; 256];
    let mut buf_next = vec![1.0_f32; 256];
    At3GainProcessor::modulate(&gi, &mut buf_cur, &mut buf_next);

    let scale = gain_level_at(2);
    for (i, v) in buf_cur.iter().enumerate().skip(40) {
        assert!((v - input / scale).abs() < 1.0e-6, "buf_cur at {i}");
    }
}

// ============================================================================
// Demodulate tests
// ============================================================================

fn demodulate(gi_now: &[GainPoint], gi_next: &[GainPoint], cur: &[f32], prev: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0_f32; 256];
    At3GainProcessor::demodulate(gi_now, gi_next, &mut out, cur, prev);
    out
}

#[test]
fn demodulate_both_empty_simple_overlap_add() {
    let out = demodulate(&[], &[], &vec![3.0; 256], &vec![5.0; 256]);
    for (i, v) in out.iter().enumerate() {
        assert!((v - 8.0).abs() < 1.0e-6, "at {i}");
    }
}

#[test]
fn demodulate_scale_from_gi_next_applied() {
    let gi_next = [gp(2, 0)];
    let out = demodulate(&[], &gi_next, &vec![3.0; 256], &vec![5.0; 256]);
    for (i, v) in out.iter().enumerate() {
        assert!((v - 17.0).abs() < 1.0e-6, "at {i}");
    }
}

#[test]
fn demodulate_gain_now_constant_region_level_applied() {
    let gi_now = [gp(2, 31)];
    let cur_val = 2.0_f32;
    let prev_val = 1.0_f32;
    let out = demodulate(&gi_now, &[], &vec![cur_val; 256], &vec![prev_val; 256]);
    let level = gain_level_at(2);
    for (i, v) in out.iter().enumerate().take(248) {
        assert!((v - (cur_val + prev_val) * level).abs() < 1.0e-5, "at {i}");
    }
}

#[test]
fn demodulate_gain_now_remainder_no_level_multiplication() {
    let gi_now = [gp(2, 4)];
    let out = demodulate(&gi_now, &[], &vec![2.0; 256], &vec![3.0; 256]);
    for (i, v) in out.iter().enumerate().skip(40) {
        assert!((v - 5.0).abs() < 1.0e-6, "at {i}");
    }
}

#[test]
fn demodulate_both_non_empty_scale_and_level_combined() {
    let gi_now = [gp(2, 31)];
    let gi_next = [gp(1, 0)];
    let out = demodulate(&gi_now, &gi_next, &vec![2.0; 256], &vec![1.0; 256]);
    let scale = gain_level_at(1);
    let level = gain_level_at(2);
    for (i, v) in out.iter().enumerate().take(248) {
        assert!((v - (2.0 * scale + 1.0) * level).abs() < 1.0e-5, "at {i}");
    }
}

// ============================================================================
// Mirror tests
// ============================================================================

#[test]
fn mirror_neutral_gain_equals_simple_overlap_add() {
    let gi = [gp(4, 31)];
    let b_cur = 3.0_f32;
    let b_next = 5.0_f32;
    let mut buf_cur = vec![b_cur; 256];
    let mut buf_next = vec![b_next; 256];

    At3GainProcessor::modulate(&gi, &mut buf_cur, &mut buf_next);
    let out = demodulate(&gi, &gi, &buf_next, &buf_cur);

    for (i, v) in out.iter().enumerate().take(248) {
        assert!((v - (b_next + b_cur)).abs() < 1.0e-5, "at {i}");
    }
}

#[test]
fn mirror_constant_region_algebraic_identity() {
    let gi = [gp(2, 31)];
    let scale = gain_level_at(2);
    let b_cur = 4.0_f32;
    let b_next = 8.0_f32;
    let mut buf_cur = vec![b_cur; 256];
    let mut buf_next = vec![b_next; 256];

    At3GainProcessor::modulate(&gi, &mut buf_cur, &mut buf_next);
    let out = demodulate(&gi, &gi, &buf_next, &buf_cur);

    let expected = b_next * scale + b_cur;
    for (i, v) in out.iter().enumerate().take(248) {
        assert!((v - expected).abs() < 1.0e-5, "at {i}");
    }
}

#[test]
fn mirror_remainder_region_algebraic_identity() {
    let gi = [gp(2, 4)];
    let scale = gain_level_at(2);
    let b_cur = 8.0_f32;
    let b_next = 4.0_f32;
    let mut buf_cur = vec![b_cur; 256];
    let mut buf_next = vec![b_next; 256];

    At3GainProcessor::modulate(&gi, &mut buf_cur, &mut buf_next);
    let out = demodulate(&gi, &gi, &buf_next, &buf_cur);

    let expected = b_next * scale + b_cur / scale;
    for (i, v) in out.iter().enumerate().skip(40) {
        assert!((v - expected).abs() < 1.0e-5, "at {i}");
    }
}

#[test]
fn mirror_two_points_constant_segments_identity() {
    let gi = [gp(0, 4), gp(2, 20)];
    let scale = gain_level_at(0);
    let b_cur = 16.0_f32;
    let b_next = 8.0_f32;
    let mut buf_cur = vec![b_cur; 256];
    let mut buf_next = vec![b_next; 256];

    At3GainProcessor::modulate(&gi, &mut buf_cur, &mut buf_next);
    let out = demodulate(&gi, &gi, &buf_next, &buf_cur);

    // First constant region [0, 32): level = GainLevel[0] = scale.
    let lev0 = gain_level_at(0);
    let expected0 = b_next * scale + b_cur * lev0 / scale;
    for (i, v) in out.iter().enumerate().take(32) {
        assert!((v - expected0).abs() < 1.0e-4, "first constant at {i}");
    }

    // Second constant region [40, 160): level = GainLevel[2].
    let lev1 = gain_level_at(2);
    let expected1 = b_next * scale + b_cur * lev1 / scale;
    for (i, v) in out.iter().enumerate().take(160).skip(40) {
        assert!((v - expected1).abs() < 1.0e-4, "second constant at {i}");
    }

    // Remainder [168, 256): out = B_next * scale + B_cur / scale.
    let expected2 = b_next * scale + b_cur / scale;
    for (i, v) in out.iter().enumerate().skip(168) {
        assert!((v - expected2).abs() < 1.0e-4, "remainder at {i}");
    }
}

// ============================================================================
// Frequency-domain helpers
// ============================================================================

fn new_bands() -> [Vec<f32>; NUM_QMF] {
    [
        vec![0.0; BAND_SZ],
        vec![0.0; BAND_SZ],
        vec![0.0; BAND_SZ],
        vec![0.0; BAND_SZ],
    ]
}

fn hf_energy(specs: &[f32], hf_start: usize) -> f32 {
    specs[hf_start..256].iter().map(|x| x * x).sum()
}

fn sine_at(i: usize, f: f32) -> f32 {
    (std::f32::consts::FRAC_PI_2 * i as f32 * f).sin()
}

// ============================================================================
// Frequency-domain runFrames / roundtrip helpers
// ============================================================================

/// Mirrors the C++ `runFrames(withModulation)` closure: runs the three-frame
/// sequence (frame 0 primes overlap, frame 1 + frame 2 are measured) and
/// returns `(specs1, specs2)` — the 1024-bin spectra for frames 1 and 2.
///
/// `f1_curve` / `f2_curve` are the band-0 modulation curves applied to frames 1
/// and 2 when `with_modulation` is true. An empty curve means "plain MDCT".
fn run_frames(
    signal: &[f32],
    with_modulation: bool,
    f1_curve: &[GainPoint],
    f2_curve: &[GainPoint],
) -> (Vec<f32>, Vec<f32>) {
    let mut mdct = Atrac3Mdct::new();
    let mut bands = new_bands();
    let mut specs1 = vec![0.0_f32; NUM_SAMPLES];
    let mut specs2 = vec![0.0_f32; NUM_SAMPLES];

    let run = |mdct: &mut Atrac3Mdct,
               bands: &mut [Vec<f32>; NUM_QMF],
               specs: &mut [f32],
               src: &[f32],
               curve: &[GainPoint]| {
        bands[0][HALF..HALF + HALF].copy_from_slice(src);
        let gain: [&[GainPoint]; NUM_QMF] = [curve, &[], &[], &[]];
        let [b0, b1, b2, b3] = bands;
        let mut refs = [
            b0.as_mut_slice(),
            b1.as_mut_slice(),
            b2.as_mut_slice(),
            b3.as_mut_slice(),
        ];
        mdct.mdct_with_gain(specs, &mut refs, &gain);
    };

    // Frame 0: prime overlap (never modulated).
    run(&mut mdct, &mut bands, &mut specs1, &signal[..HALF], &[]);

    // Frame 1.
    let f1: &[GainPoint] = if with_modulation { f1_curve } else { &[] };
    run(
        &mut mdct,
        &mut bands,
        &mut specs1,
        &signal[HALF..2 * HALF],
        f1,
    );

    // Frame 2.
    let f2: &[GainPoint] = if with_modulation { f2_curve } else { &[] };
    run(
        &mut mdct,
        &mut bands,
        &mut specs2,
        &signal[2 * HALF..3 * HALF],
        f2,
    );

    (specs1, specs2)
}

/// Mirrors the C++ roundtrip block: for each of the three frames, run
/// `Mdct(Modulate)` then `Midct(Demodulate)` with the per-frame band-0 curves.
/// Frame 1 modulates with `f1`, frame 2 with `f2`. The demodulation pairing
/// follows the C++ `(siCur, siNext)` schedule:
///   frame 1: Demodulate(cur=[],  next=f1)
///   frame 2: Demodulate(cur=f1,  next=f2)
/// Returns `signal_res` (length 3*HALF).
fn roundtrip(signal: &[f32], f1: &[GainPoint], f2: &[GainPoint]) -> Vec<f32> {
    let mut mdct = Atrac3Mdct::new();
    let mut enc = new_bands();
    let mut dec = new_bands();
    let mut signal_res = vec![0.0_f32; 3 * HALF];
    let mut sp = vec![0.0_f32; NUM_SAMPLES];

    for frame in 0..3 {
        enc[0][HALF..HALF + HALF].copy_from_slice(&signal[frame * HALF..frame * HALF + HALF]);

        // --- Encode ---
        let mod_curve: &[GainPoint] = match frame {
            1 => f1,
            2 => f2,
            _ => &[],
        };
        {
            let gain: [&[GainPoint]; NUM_QMF] = [mod_curve, &[], &[], &[]];
            let [b0, b1, b2, b3] = &mut enc;
            let mut refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.mdct_with_gain(&mut sp, &mut refs, &gain);
        }

        // --- Decode ---
        let (cur_curve, next_curve): (&[GainPoint], &[GainPoint]) = match frame {
            1 => (&[], f1),
            2 => (f1, f2),
            _ => (&[], &[]),
        };
        {
            let demod: [(&[GainPoint], &[GainPoint]); NUM_QMF] =
                [(cur_curve, next_curve), (&[], &[]), (&[], &[]), (&[], &[])];
            let [d0, d1, d2, d3] = &mut dec;
            let mut refs = [
                d0.as_mut_slice(),
                d1.as_mut_slice(),
                d2.as_mut_slice(),
                d3.as_mut_slice(),
            ];
            mdct.midct_with_gain(&mut sp, &mut refs, &demod);
        }

        signal_res[frame * HALF..frame * HALF + HALF].copy_from_slice(&dec[0][..HALF]);
    }

    signal_res
}

fn hf_e(specs: &[f32], hf_start: usize) -> f32 {
    hf_energy(specs, hf_start)
}

fn assert_roundtrip(signal: &[f32], signal_res: &[f32]) {
    for i in HALF..3 * HALF {
        assert!(
            (signal[i - HALF] - signal_res[i]).abs() <= 0.000_01,
            "roundtrip mismatch at {i}: {} vs {}",
            signal[i - HALF],
            signal_res[i]
        );
    }
}

// ============================================================================
// Frequency-domain energy-reduction tests
// ============================================================================

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy() {
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let f = 0.125_f32;
    let gain_inc = 2.0_f32.powf(3.0 / 8.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_quiet * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + k] = a_quiet * g * sine_at(HALF + k, f);
            g *= gain_inc;
        }
    }
    for i in HALF + 8..HALF * 2 {
        signal[i] = a_loud * sine_at(i, f);
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_loud * sine_at(i, f);
    }

    let f1 = vec![gp(7, 0)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 30;
    let hf_nomod = hf_e(&s1n, hf_start);
    let hf_mod = hf_e(&s1m, hf_start);
    assert!(hf_mod * 10.0 < hf_nomod);
    assert!(hf_nomod > 0.0);

    let hf2_nomod = hf_e(&s2n, hf_start);
    let hf2_mod = hf_e(&s2m, hf_start);
    assert!(hf2_mod <= hf2_nomod);

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_transient_in_frame() {
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let f = 0.125_f32;
    let gain_inc_rel = 2.0_f32.powf(-3.0 / 8.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_loud * sine_at(i, f);
    }
    for i in HALF..HALF + 64 {
        signal[i] = a_loud * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 64 + k] = a_loud * g * sine_at(HALF + 64 + k, f);
            g *= gain_inc_rel;
        }
    }
    for i in HALF + 72..HALF * 2 {
        signal[i] = a_quiet * sine_at(i, f);
    }

    for i in HALF * 2..HALF * 2 + 8 {
        signal[i] = a_loud * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF * 2 + 8 + k] = a_loud * g * sine_at(HALF * 2 + 8 + k, f);
            g *= gain_inc_rel;
        }
    }
    for i in HALF * 2 + 16..HALF * 3 {
        signal[i] = a_quiet * sine_at(i, f);
    }

    let f1 = vec![gp(4, 8), gp(7, 31)];
    let f2 = vec![gp(1, 1)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &f2);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &f2);

    let hf_start = 30;
    let hf_nomod = hf_e(&s1n, hf_start);
    let hf_mod = hf_e(&s1m, hf_start);
    assert!(hf_mod * 10.0 < hf_nomod);
    assert!(hf_nomod > 0.0);

    let hf2_nomod = hf_e(&s2n, hf_start);
    let hf2_mod = hf_e(&s2m, hf_start);
    assert!(hf2_mod <= hf2_nomod);

    let res = roundtrip(&signal, &f1, &f2);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_attack_and_release() {
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let f = 0.125_f32;
    let gain_inc_atk = 2.0_f32.powf(3.0 / 8.0);
    let gain_inc_rel = 2.0_f32.powf(-3.0 / 8.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_quiet * sine_at(i, f);
    }
    for i in HALF..HALF + 32 {
        signal[i] = a_quiet * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 32 + k] = a_quiet * g * sine_at(HALF + 32 + k, f);
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 40..HALF + 96 {
        signal[i] = a_loud * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 96 + k] = a_loud * g * sine_at(HALF + 96 + k, f);
            g *= gain_inc_rel;
        }
    }
    for i in HALF + 104..HALF * 2 {
        signal[i] = a_quiet * sine_at(i, f);
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_quiet * sine_at(i, f);
    }

    let f1 = vec![gp(4, 4), gp(1, 12)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 30;
    assert!(hf_e(&s1m, hf_start) * 10.0 < hf_e(&s1n, hf_start));
    assert!(hf_e(&s1n, hf_start) > 0.0);
    assert!(hf_e(&s2m, hf_start) * 10.0 < hf_e(&s2n, hf_start));

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_release_and_attack() {
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let f = 0.125_f32;
    let gain_inc_atk = 2.0_f32.powf(3.0 / 8.0);
    let gain_inc_rel = 2.0_f32.powf(-3.0 / 8.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_loud * sine_at(i, f);
    }
    for i in HALF..HALF + 32 {
        signal[i] = a_loud * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 32 + k] = a_loud * g * sine_at(HALF + 32 + k, f);
            g *= gain_inc_rel;
        }
    }
    for i in HALF + 40..HALF + 96 {
        signal[i] = a_quiet * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 96 + k] = a_quiet * g * sine_at(HALF + 96 + k, f);
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 104..HALF * 2 {
        signal[i] = a_loud * sine_at(i, f);
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_loud * sine_at(i, f);
    }

    let f1 = vec![gp(4, 4), gp(7, 12)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 30;
    assert!(hf_e(&s1m, hf_start) * 10.0 < hf_e(&s1n, hf_start));
    assert!(hf_e(&s1n, hf_start) > 0.0);
    assert!(hf_e(&s2m, hf_start) * 10.0 < hf_e(&s2n, hf_start));

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_dc_signal() {
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let gain_inc_atk = 2.0_f32.powf(3.0 / 8.0);
    let gain_inc_rel = 2.0_f32.powf(-3.0 / 8.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_quiet;
    }
    for i in HALF..HALF + 8 {
        signal[i] = a_quiet;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 8 + k] = g;
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 16..HALF * 2 {
        signal[i] = a_loud;
    }

    for i in HALF * 2..HALF * 2 + 8 {
        signal[i] = a_loud;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF * 2 + 8 + k] = a_loud * g;
            g *= gain_inc_rel;
        }
    }
    for i in HALF * 2 + 16..HALF * 3 {
        signal[i] = a_quiet;
    }

    let f1 = vec![gp(7, 1)];
    let f2 = vec![gp(1, 1)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &f2);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &f2);

    let hf_start = 4;
    assert!(hf_e(&s1m, hf_start) * 10.0 < hf_e(&s1n, hf_start));
    assert!(hf_e(&s1n, hf_start) > 0.0);
    assert!(hf_e(&s2m, hf_start) * 10.0 < hf_e(&s2n, hf_start));
    assert!(hf_e(&s2n, hf_start) > 0.0);

    let res = roundtrip(&signal, &f1, &f2);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_quiet_to_loud_transient() {
    let gain_inc_atk = 2.0_f32.powf(3.0 / 8.0);
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let f = 0.125_f32;

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_quiet * sine_at(i, f);
    }
    for i in HALF..HALF + 64 {
        signal[i] = a_quiet * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 56 + k] *= g;
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 64..HALF * 2 {
        signal[i] = a_loud * sine_at(i, f);
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_loud * sine_at(i, f);
    }

    let f1 = vec![gp(7, 7)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 30;
    let hf1_nomod = hf_e(&s1n, hf_start);
    let hf1_mod = hf_e(&s1m, hf_start);
    let hf2_nomod = hf_e(&s2n, hf_start);
    let hf2_mod = hf_e(&s2m, hf_start);

    assert!(hf1_mod * 10.0 < hf1_nomod);
    assert!(hf1_nomod > 0.0);
    assert!(hf2_mod * 10.0 <= hf2_nomod);
    assert!(hf2_nomod > 0.0);

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_very_quiet_to_loud_transient() {
    let gain_inc_atk = 2.0_f32.powf(11.0 / 8.0);
    let a_loud = 1.0_f32;
    let a_quiet = 2.0_f32.powf(-11.0);
    let f = 0.125_f32;

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_quiet * sine_at(i, f);
    }
    for i in HALF..HALF + 64 {
        signal[i] = a_quiet * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 56 + k] *= g;
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 64..HALF * 2 {
        signal[i] = a_loud * sine_at(i, f);
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_loud * sine_at(i, f);
    }

    let f1 = vec![gp(15, 7)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 30;
    let hf1_nomod = hf_e(&s1n, hf_start);
    let hf1_mod = hf_e(&s1m, hf_start);
    let hf2_nomod = hf_e(&s2n, hf_start);
    let hf2_mod = hf_e(&s2m, hf_start);

    assert!(hf1_mod * 10.0 < hf1_nomod);
    assert!(hf1_nomod > 0.0);
    assert!(hf2_mod * 10.0 <= hf2_nomod);
    assert!(hf2_nomod > 0.0);

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_loud_to_very_quiet_transient() {
    let gain_inc_rel = 2.0_f32.powf(-4.0 / 8.0);
    let a_loud = 1.0_f32;
    let a_quiet = 2.0_f32.powf(-4.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_loud;
    }
    for i in HALF..HALF + 64 {
        signal[i] = a_loud;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 56 + k] *= g;
            g *= gain_inc_rel;
        }
    }
    for i in HALF + 64..HALF * 2 {
        signal[i] = a_quiet;
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_quiet;
    }

    let f1 = vec![gp(0, 7)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 30;
    let hf1_nomod = hf_e(&s1n, hf_start);
    let hf1_mod = hf_e(&s1m, hf_start);
    let hf2_nomod = hf_e(&s2n, hf_start);
    let hf2_mod = hf_e(&s2m, hf_start);

    assert!(hf1_mod * 10.0 < hf1_nomod);
    assert!(hf1_nomod > 0.0);
    assert!(hf2_mod * 10.0 <= hf2_nomod);
    assert!(hf2_nomod > 0.0);

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_dc_signal2() {
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let gain_inc_atk = 2.0_f32.powf(3.0 / 8.0);
    let gain_inc_rel = 2.0_f32.powf(-3.0 / 8.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_quiet;
    }
    for i in HALF..HALF + 8 {
        signal[i] = a_quiet;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 8 + k] = g;
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 16..HALF * 2 {
        signal[i] = a_loud;
    }

    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF * 2 + k] = a_loud * g;
            g *= gain_inc_rel;
        }
    }
    for i in HALF * 2 + 8..HALF * 3 {
        signal[i] = a_quiet;
    }

    let f1 = vec![gp(7, 1)];
    let f2 = vec![gp(1, 0)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &f2);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &f2);

    let hf_start = 4;
    assert!(hf_e(&s1m, hf_start) * 10.0 < hf_e(&s1n, hf_start));
    assert!(hf_e(&s1n, hf_start) > 0.0);
    assert!(hf_e(&s2m, hf_start) * 10.0 < hf_e(&s2n, hf_start));
    assert!(hf_e(&s2n, hf_start) > 0.0);

    let res = roundtrip(&signal, &f1, &f2);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_2points_without_scale_dc2() {
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let gain_inc_atk = 2.0_f32.powf(3.0 / 8.0);
    let gain_inc_rel = 2.0_f32.powf(-3.0 / 8.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_quiet;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + k] = g;
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 8..HALF + 248 {
        signal[i] = a_loud;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 248 + k] = a_loud * g;
            g *= gain_inc_rel;
        }
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_quiet;
    }

    let f1 = vec![gp(4, 0), gp(1, 31)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 4;
    assert!(hf_e(&s1m, hf_start) * 10.0 < hf_e(&s1n, hf_start));
    assert!(hf_e(&s1n, hf_start) > 0.0);
    assert!(hf_e(&s2m, hf_start) * 10.0 < hf_e(&s2n, hf_start));
    assert!(hf_e(&s2n, hf_start) > 0.0);

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_2points_without_scale_dc_rel29() {
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let gain_inc_atk = 2.0_f32.powf(3.0 / 8.0);
    let gain_inc_rel = 2.0_f32.powf(-3.0 / 8.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_quiet;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + k] = g;
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 8..HALF + 232 {
        signal[i] = a_loud;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 232 + k] = a_loud * g;
            g *= gain_inc_rel;
        }
    }
    for i in HALF + 240..HALF * 2 {
        signal[i] = a_quiet;
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_quiet;
    }

    let f1 = vec![gp(4, 0), gp(1, 29)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 4;
    assert!(hf_e(&s1m, hf_start) * 10.0 < hf_e(&s1n, hf_start));
    assert!(hf_e(&s1n, hf_start) > 0.0);
    assert!(hf_e(&s2m, hf_start) * 10.0 < hf_e(&s2n, hf_start));
    assert!(hf_e(&s2n, hf_start) > 0.0);

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_2points_without_scale_dc_rel30() {
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let gain_inc_atk = 2.0_f32.powf(3.0 / 8.0);
    let gain_inc_rel = 2.0_f32.powf(-3.0 / 8.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_quiet;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + k] = g;
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 8..HALF + 240 {
        signal[i] = a_loud;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 240 + k] = a_loud * g;
            g *= gain_inc_rel;
        }
    }
    for i in HALF + 248..HALF * 2 {
        signal[i] = a_quiet;
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_quiet;
    }

    let f1 = vec![gp(4, 0), gp(1, 30)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 4;
    assert!(hf_e(&s1m, hf_start) * 10.0 < hf_e(&s1n, hf_start));
    assert!(hf_e(&s1n, hf_start) > 0.0);
    assert!(hf_e(&s2m, hf_start) * 10.0 < hf_e(&s2n, hf_start));
    assert!(hf_e(&s2n, hf_start) > 0.0);

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_hole_in_loud() {
    let a_loud = 8.0_f32;
    let a_quiet = 1.0_f32;
    let a_hole = 0.125_f32;
    let gain_inc_atk = 2.0_f32.powf(3.0 / 8.0);
    let gain_inc_rel = 2.0_f32.powf(-3.0 / 8.0);
    let gain_inc_hole_down = 2.0_f32.powf(-3.0 / 4.0);
    let gain_inc_hole_up = 2.0_f32.powf(3.0 / 4.0);

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_quiet;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + k] = g;
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 8..HALF + 104 {
        signal[i] = a_loud;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 104 + k] = a_loud * g;
            g *= gain_inc_hole_down;
        }
    }
    for i in HALF + 112..HALF + 144 {
        signal[i] = a_hole;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 144 + k] = a_hole * g;
            g *= gain_inc_hole_up;
        }
    }
    for i in HALF + 152..HALF + 232 {
        signal[i] = a_loud;
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 232 + k] = a_loud * g;
            g *= gain_inc_rel;
        }
    }
    for i in HALF + 240..HALF * 2 {
        signal[i] = a_quiet;
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_quiet;
    }

    let f1 = vec![gp(4, 0), gp(1, 13), gp(7, 18), gp(1, 29)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 4;
    assert!(hf_e(&s1m, hf_start) * 10.0 < hf_e(&s1n, hf_start));
    assert!(hf_e(&s1n, hf_start) > 0.0);
    assert!(hf_e(&s2m, hf_start) * 10.0 < hf_e(&s2n, hf_start));
    assert!(hf_e(&s2n, hf_start) > 0.0);

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

#[test]
fn freq_domain_gain_modulation_reduces_spectral_energy_attack_and_release_level_rise() {
    let a_before = 1.0_f32;
    let a_loud = 8.0_f32;
    let a_after = 2.0_f32;
    let f = 0.125_f32;
    let gain_inc_atk = 2.0_f32.powf(3.0 / 8.0);
    let gain_inc_rel = 0.840896_f32;

    let mut signal = vec![0.0_f32; HALF * 3];

    for i in 0..HALF {
        signal[i] = a_before * sine_at(i, f);
    }
    for i in HALF..HALF + 32 {
        signal[i] = a_before * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 32 + k] = a_before * g * sine_at(HALF + 32 + k, f);
            g *= gain_inc_atk;
        }
    }
    for i in HALF + 40..HALF + 96 {
        signal[i] = a_loud * sine_at(i, f);
    }
    {
        let mut g = 1.0_f32;
        for k in 0..8 {
            signal[HALF + 96 + k] = a_loud * g * sine_at(HALF + 96 + k, f);
            g *= gain_inc_rel;
        }
    }
    for i in HALF + 104..HALF * 2 {
        signal[i] = a_after * sine_at(i, f);
    }
    for i in HALF * 2..HALF * 3 {
        signal[i] = a_after * sine_at(i, f);
    }

    let f1 = vec![gp(5, 4), gp(2, 12)];
    let (s1n, s2n) = run_frames(&signal, false, &f1, &[]);
    let (s1m, s2m) = run_frames(&signal, true, &f1, &[]);

    let hf_start = 30;
    assert!(hf_e(&s1m, hf_start) * 10.0 < hf_e(&s1n, hf_start));
    assert!(hf_e(&s1n, hf_start) > 0.0);
    assert!(hf_e(&s2m, hf_start) * 10.0 < hf_e(&s2n, hf_start));

    let res = roundtrip(&signal, &f1, &[]);
    assert_roundtrip(&signal, &res);
}

// ============================================================================
// BoundaryLevelMismatch (Issue #1) tests
// ============================================================================

/// Deterministic pseudo-random burst signal used by the roundtrip tests,
/// mirroring the C++ LCG generator exactly.
fn make_burst_signal(total_samples: usize, event_dist: i32) -> Vec<f32> {
    const SAMPLE_RATE: f32 = 11025.0;
    const CARRIER_HZ: f32 = 1500.0;
    const BASE_AMP: f32 = 0.1;
    const BURST_AMP_LO: f32 = 0.3;
    const BURST_AMP_HI: f32 = 0.9;

    let mut signal = vec![0.0_f32; total_samples];
    for s in 0..total_samples {
        signal[s] =
            BASE_AMP * (2.0 * std::f32::consts::PI * CARRIER_HZ * s as f32 / SAMPLE_RATE).sin();
    }

    let mut lcg: u32 = 0xdead_beef;
    let mut next_lcg = || {
        lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        lcg
    };

    let mut pos = event_dist;
    while pos + event_dist < total_samples as i32 {
        let burst_len = 8 + (next_lcg() >> 24) as i32 % 249;
        let burst_amp =
            BURST_AMP_LO + (BURST_AMP_HI - BURST_AMP_LO) * (next_lcg() & 0xff) as f32 / 255.0;
        let end = std::cmp::min(pos + burst_len, total_samples as i32);
        for s in pos..end {
            signal[s as usize] = burst_amp
                * (2.0 * std::f32::consts::PI * CARRIER_HZ * s as f32 / SAMPLE_RATE).sin();
        }
        pos += burst_len + event_dist + (next_lcg() >> 16) as i32 % (event_dist / 4);
    }

    signal
}

#[test]
fn boundary_level_mismatch_issue1_false_transient_on_constant_tone_after_onset() {
    let sample_rate = 11025.0_f32;
    let low_cut_hz = 600.0_f32;
    let freq = 2000.0_f32;
    let amplitude = 0.5_f32;

    let mut upsampler = SpectralUpsampler::with_default_eps(sample_rate, low_cut_hz);

    // --- Call N: current frame = silence, lookahead = onset of tone ---
    let mut input1 = vec![0.0_f32; 512];
    for i in 0..128 {
        input1[384 + i] =
            amplitude * (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin();
    }
    let result1 = upsampler.process(&input1);
    let saved_last_level = analyze_gain(&result1.signal[3072..3072 + 64], 1, true, None, None)[0];

    // --- Call N+1: current frame = tone (onset at sample 0), lookahead = more tone ---
    let mut input2 = vec![0.0_f32; 512];
    for i in 0..384 {
        input2[128 + i] =
            amplitude * (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin();
    }
    let result2 = upsampler.process(&input2);
    let in0 = analyze_gain(&result2.signal[1024..1024 + 64], 1, true, None, None)[0];

    let lo = saved_last_level.min(in0);
    let hi = saved_last_level.max(in0);
    let ratio = hi / lo.max(1e-9);

    assert!(
        ratio < 2.0,
        "Boundary amplitude mismatch: savedLastLevel={saved_last_level} in0={in0} ratio={ratio}"
    );

    let mut ctx = CurveBuilderCtx {
        last_level: saved_last_level,
        ..Default::default()
    };
    let gain = analyze_gain(&result2.signal[1024..1024 + 2048], 32, true, None, None);
    let next_level = analyze_gain(&result2.signal[3072..3072 + 64], 1, true, None, None)[0];
    let curve = calc_curve(&gain, &mut ctx, Some(next_level), 2.0, None, None, None);

    assert!(
        curve.is_empty(),
        "False boundary transient emitted: {:?}",
        curve.first()
    );
}

#[test]
fn boundary_level_mismatch_issue1_mdct_roundtrip_no_gain() {
    const MIN_EVENT_DIST: i32 = 512;
    const TOTAL_SAMPLES: usize = (MIN_EVENT_DIST * 64) as usize;
    const FRAME_SZ: usize = 256;
    const NUM_FRAMES: usize = TOTAL_SAMPLES / FRAME_SZ;

    let signal = make_burst_signal(TOTAL_SAMPLES, MIN_EVENT_DIST);

    let mut mdct = Atrac3Mdct::new();
    let mut enc = new_bands();
    let mut dec = new_bands();
    let mut sp = vec![0.0_f32; NUM_SAMPLES];
    let mut reconstructed = vec![0.0_f32; TOTAL_SAMPLES];

    for frame in 0..NUM_FRAMES {
        enc[0][FRAME_SZ..FRAME_SZ + FRAME_SZ]
            .copy_from_slice(&signal[frame * FRAME_SZ..frame * FRAME_SZ + FRAME_SZ]);
        {
            let [b0, b1, b2, b3] = &mut enc;
            let mut refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.mdct(&mut sp, &mut refs);
        }
        {
            let [d0, d1, d2, d3] = &mut dec;
            let mut refs = [
                d0.as_mut_slice(),
                d1.as_mut_slice(),
                d2.as_mut_slice(),
                d3.as_mut_slice(),
            ];
            mdct.midct(&mut sp, &mut refs);
        }
        if frame >= 1 {
            reconstructed[(frame - 1) * FRAME_SZ..(frame - 1) * FRAME_SZ + FRAME_SZ]
                .copy_from_slice(&dec[0][..FRAME_SZ]);
        }
    }

    let skip_frames = 1;
    let err_limit = 1e-5_f32;
    let mut max_err = 0.0_f32;
    for frame in skip_frames..=NUM_FRAMES - 2 {
        for s in 0..FRAME_SZ {
            let err = (reconstructed[frame * FRAME_SZ + s] - signal[frame * FRAME_SZ + s]).abs();
            if err > max_err {
                max_err = err;
            }
        }
    }

    assert!(
        max_err < err_limit,
        "Pure MDCT->IMDCT roundtrip error {max_err} exceeds {err_limit}"
    );
}

/// Runs the upsampler -> calc_curve -> Modulate -> MDCT -> (optional quantize)
/// -> IMDCT -> Demodulate pipeline for a burst signal, returning the max
/// per-sample reconstruction error over the interior frames.
fn run_gain_roundtrip(total_samples: usize, event_dist: i32, quant_step: Option<f32>) -> f32 {
    const FRAME_SZ: usize = 256;
    const SAMPLE_RATE: f32 = 11025.0;
    const LOW_CUT_HZ: f32 = 600.0;
    let num_frames = total_samples / FRAME_SZ;

    let signal = make_burst_signal(total_samples, event_dist);

    let mut mdct = Atrac3Mdct::new();
    let mut upsampler = SpectralUpsampler::with_default_eps(SAMPLE_RATE, LOW_CUT_HZ);
    let mut ctx = CurveBuilderCtx::default();

    let mut enc = new_bands();
    let mut dec = new_bands();
    let mut sp = vec![0.0_f32; NUM_SAMPLES];

    let mut look_ahead = vec![0.0_f32; 512];

    let mut si_prev: Vec<GainPoint>;
    let mut si_cur: Vec<GainPoint> = Vec::new();

    let mut reconstructed = vec![0.0_f32; total_samples];

    for frame in 0..num_frames {
        let cur_frm = &signal[frame * FRAME_SZ..frame * FRAME_SZ + FRAME_SZ];

        // --- Update upsampler window ---
        look_ahead[128..128 + FRAME_SZ].copy_from_slice(cur_frm);
        if frame + 1 < num_frames {
            look_ahead[384..384 + 128]
                .copy_from_slice(&signal[(frame + 1) * FRAME_SZ..(frame + 1) * FRAME_SZ + 128]);
        } else {
            for v in look_ahead[384..384 + 128].iter_mut() {
                *v = 0.0;
            }
        }

        // --- Gain curve (mirrors CreateSubbandInfo for band 0) ---
        si_prev = std::mem::take(&mut si_cur);
        si_cur = Vec::new();

        let result = upsampler.process(&look_ahead);
        if result.high_freq_ratio >= SpectralUpsampler::HIGH_FREQ_THRESHOLD {
            let gain = analyze_gain(&result.signal[1024..1024 + 2048], 32, true, None, None);
            let next_level = analyze_gain(&result.signal[3072..3072 + 64], 1, true, None, None)[0];
            let curve_points = calc_curve(&gain, &mut ctx, Some(next_level), 2.0, None, None, None);
            if !curve_points.is_empty() {
                si_cur = curve_points
                    .iter()
                    .map(|p| GainPoint {
                        level: p.level,
                        location: p.location,
                    })
                    .collect();
            }
        } else {
            ctx.last_level = 0.0;
        }

        // --- Encode: Modulate(si_cur) -> MDCT ---
        enc[0][FRAME_SZ..FRAME_SZ + FRAME_SZ].copy_from_slice(cur_frm);
        {
            let gain: [&[GainPoint]; NUM_QMF] = [&si_cur, &[], &[], &[]];
            let [b0, b1, b2, b3] = &mut enc;
            let mut refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.mdct_with_gain(&mut sp, &mut refs, &gain);
        }

        // --- Quantize band 0 spectral coefficients ---
        if let Some(step) = quant_step {
            for k in 0..256 {
                sp[k] = (sp[k] / step).round() * step;
            }
        }

        // --- Decode: IMDCT -> Demodulate(si_prev, si_cur) ---
        {
            let demod: [(&[GainPoint], &[GainPoint]); NUM_QMF] =
                [(&si_prev, &si_cur), (&[], &[]), (&[], &[]), (&[], &[])];
            let [d0, d1, d2, d3] = &mut dec;
            let mut refs = [
                d0.as_mut_slice(),
                d1.as_mut_slice(),
                d2.as_mut_slice(),
                d3.as_mut_slice(),
            ];
            mdct.midct_with_gain(&mut sp, &mut refs, &demod);
        }

        if frame >= 1 {
            reconstructed[(frame - 1) * FRAME_SZ..(frame - 1) * FRAME_SZ + FRAME_SZ]
                .copy_from_slice(&dec[0][..FRAME_SZ]);
        }

        // Advance upsampler window: old [256..511] becomes [0..255].
        look_ahead.copy_within(256..512, 0);
    }

    let mut max_err = 0.0_f32;
    for frame in 1..=num_frames - 2 {
        for s in 0..FRAME_SZ {
            let e = (reconstructed[frame * FRAME_SZ + s] - signal[frame * FRAME_SZ + s]).abs();
            if e > max_err {
                max_err = e;
            }
        }
    }
    max_err
}

#[test]
fn boundary_level_mismatch_issue1_mdct_roundtrip_with_gain() {
    const MIN_EVENT_DIST: i32 = 512;
    const TOTAL_SAMPLES: usize = (MIN_EVENT_DIST * 64) as usize;
    let err_limit = 1e-4_f32;
    let max_err = run_gain_roundtrip(TOTAL_SAMPLES, MIN_EVENT_DIST, None);
    assert!(
        max_err < err_limit,
        "MDCT->IMDCT roundtrip WITH gain modulation, error {max_err} exceeds {err_limit}"
    );
}

#[test]
fn boundary_level_mismatch_issue1_roundtrip_with_gain_and_quantization() {
    const MIN_EVENT_DIST: i32 = 512;
    const TOTAL_SAMPLES: usize = (MIN_EVENT_DIST * 64) as usize;
    let quant_step = 1e-3_f32;
    let err_limit = quant_step * 400.0;
    let max_err = run_gain_roundtrip(TOTAL_SAMPLES, MIN_EVENT_DIST, Some(quant_step));
    assert!(
        max_err < err_limit,
        "MDCT->IMDCT+quantization roundtrip max error {max_err} exceeds {err_limit}"
    );
}
