//! Port of the gain-compensation `TAtrac3MDCT` cases from
//! `atracdenc/src/atrac3denc_ut.cpp`.
//!
//! Each test drives a multi-frame MDCT -> IMDCT roundtrip over band 0, applying
//! a gain-modulation curve at frame position 1024 (and sometimes 1024+256) on
//! the forward transform and the matching demodulation curve(s) on the inverse
//! transform, then asserts the decoded signal reconstructs the (delayed) input.

use atracdenc_core::{
    at3::{
        data::{NUM_QMF, NUM_SAMPLES},
        mdct::Atrac3Mdct,
    },
    dsp::gain::GainPoint,
};

const WORK_SZ: usize = 256;
const BAND_BUF_SZ: usize = 512;

fn gp(level: u32, location: u32) -> GainPoint {
    GainPoint { level, location }
}

fn assert_near_delayed(signal: &[f32], signal_res: &[f32], eps: f32) {
    for i in WORK_SZ..signal.len() {
        assert!(
            (signal[i - WORK_SZ] - signal_res[i]).abs() <= eps,
            "idx {i}: {} != {}, eps {eps}",
            signal[i - WORK_SZ],
            signal_res[i]
        );
    }
}

/// Per-frame gain schedule. For a given frame start position it returns the
/// modulation curve (for `mdct`) and the (cur, next) demodulation curves (for
/// `midct`).
struct GainSchedule {
    modulate: Box<dyn Fn(usize) -> Vec<GainPoint>>,
    demodulate: Box<dyn Fn(usize) -> (Vec<GainPoint>, Vec<GainPoint>)>,
}

/// Runs the standard `TAtrac3MDCT` roundtrip harness over band 0.
///
/// When `capture_specs` is set, the first 256 MDCT coefficients of the frame
/// at that position are returned; this mirrors the C++ `mdctResult*` captures.
fn run_roundtrip(
    signal: &[f32],
    schedule: &GainSchedule,
    capture: &[usize],
) -> (Vec<f32>, Vec<Vec<f32>>) {
    let len = signal.len();
    let mut mdct = Atrac3Mdct::new();
    let mut signal_res = vec![0.0_f32; len];

    let mut enc_bands = [
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
    ];
    let mut dec_bands = [
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
    ];

    let mut captured = vec![Vec::new(); capture.len()];

    for pos in (0..len).step_by(WORK_SZ) {
        enc_bands[0][WORK_SZ..WORK_SZ + WORK_SZ].copy_from_slice(&signal[pos..pos + WORK_SZ]);
        let mut specs = vec![0.0_f32; NUM_SAMPLES];

        let mod_curve = (schedule.modulate)(pos);
        let gain: [&[GainPoint]; NUM_QMF] = [&mod_curve, &[], &[], &[]];
        {
            let [b0, b1, b2, b3] = &mut enc_bands;
            let mut enc_refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.mdct_with_gain(&mut specs, &mut enc_refs, &gain);
        }

        if let Some(slot) = capture.iter().position(|p| *p == pos) {
            captured[slot] = specs[..256].to_vec();
        }

        let (cur, next) = (schedule.demodulate)(pos);
        let demod: [(&[GainPoint], &[GainPoint]); NUM_QMF] =
            [(&cur, &next), (&[], &[]), (&[], &[]), (&[], &[])];
        {
            let [b0, b1, b2, b3] = &mut dec_bands;
            let mut dec_refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.midct_with_gain(&mut specs, &mut dec_refs, &demod);
        }

        signal_res[pos..pos + WORK_SZ].copy_from_slice(&dec_bands[0][..WORK_SZ]);
    }

    (signal_res, captured)
}

/// Single gain point applied at pos==1024 with the standard demod schedule
/// (next at 1024, cur at 1280). Mirrors the `Gain1Point*Dc` tests.
fn one_point_dc(point: GainPoint, eps: f32) {
    let len = 2048;
    let signal = vec![1.0_f32; len];

    let p = point;
    let schedule = GainSchedule {
        modulate: Box::new(move |pos| if pos == 1024 { vec![p] } else { vec![] }),
        demodulate: Box::new(move |pos| match pos {
            1024 => (vec![], vec![p]),
            1280 => (vec![p], vec![]),
            _ => (vec![], vec![]),
        }),
    };

    let (signal_res, _) = run_roundtrip(&signal, &schedule, &[]);
    assert_near_delayed(&signal, &signal_res, eps);
}

/// Two gain points applied at pos==1024. Mirrors the `Gain2*Dc` tests.
fn two_points_dc(curve: Vec<GainPoint>, eps: f32) {
    let len = 2048;
    let signal = vec![1.0_f32; len];

    let c = curve;
    let schedule = GainSchedule {
        modulate: Box::new({
            let c = c.clone();
            move |pos| if pos == 1024 { c.clone() } else { vec![] }
        }),
        demodulate: Box::new(move |pos| match pos {
            1024 => (vec![], c.clone()),
            1280 => (c.clone(), vec![]),
            _ => (vec![], vec![]),
        }),
    };

    let (signal_res, _) = run_roundtrip(&signal, &schedule, &[]);
    assert_near_delayed(&signal, &signal_res, eps);
}

#[test]
fn atrac3_mdct_gain1_point_at_end_dc() {
    one_point_dc(gp(3, 31), 2.0e-6);
}

#[test]
fn atrac3_mdct_gain1_point_at_start_dc() {
    one_point_dc(gp(3, 0), 2.0e-6);
}

#[test]
fn atrac3_mdct_gain2_points_dc() {
    two_points_dc(vec![gp(3, 2), gp(2, 5)], 2.0e-6);
}

#[test]
fn atrac3_mdct_gain2_near_points_dc() {
    two_points_dc(vec![gp(3, 2), gp(3, 3)], 2.0e-6);
}

/// Shape the signal with a `gaininc` ramp so the gain curve fully compensates
/// the synthetic transient; afterwards the per-frame spectra must match the
/// captured reference (`mdctResult1`). Mirrors `*CompensateWithoutScaleDc*`.
fn compensate_without_scale(
    ramp_segments: &[(usize, usize, RampOp)],
    curve: Vec<GainPoint>,
    spec_eps: f32,
    sig_eps: f32,
) {
    let len = 2048;
    let mut signal = vec![1.0_f32; len];
    apply_ramp(&mut signal, ramp_segments);

    let c = curve;
    let schedule = GainSchedule {
        modulate: Box::new({
            let c = c.clone();
            move |pos| if pos == 1024 { c.clone() } else { vec![] }
        }),
        demodulate: Box::new(move |pos| match pos {
            1024 => (vec![], c.clone()),
            1280 => (c.clone(), vec![]),
            _ => (vec![], vec![]),
        }),
    };

    // Capture the modulated reference spectrum at pos==1024, then re-run so we
    // can compare every later frame's spectrum to it (the C++ test inspects
    // specs in-line; here we capture once and assert below via a full pass).
    let (signal_res, captured) = run_roundtrip_with_spec_check(&signal, &schedule, 1024, spec_eps);
    let _ = captured;
    assert_near_delayed(&signal, &signal_res, sig_eps);
}

enum RampOp {
    MulInc,
    Hold,
    DivInc,
}

const GAIN_INC: f32 = 1.296_84;

fn apply_ramp(signal: &mut [f32], segments: &[(usize, usize, RampOp)]) {
    // Segments are applied with a single running `level` accumulator, exactly
    // like the C++ nested loops.
    let mut level = 1.0_f32;
    for (start, end, op) in segments {
        for s in signal.iter_mut().take(*end).skip(*start) {
            match op {
                RampOp::MulInc => {
                    *s *= level;
                    level *= GAIN_INC;
                }
                RampOp::Hold => {
                    *s *= level;
                }
                RampOp::DivInc => {
                    *s *= level;
                    level /= GAIN_INC;
                }
            }
        }
    }
}

/// Variant of `run_roundtrip` that captures the modulated spectrum at
/// `ref_pos` and asserts every frame at `>= ref_pos + 256` matches it.
fn run_roundtrip_with_spec_check(
    signal: &[f32],
    schedule: &GainSchedule,
    ref_pos: usize,
    spec_eps: f32,
) -> (Vec<f32>, Vec<f32>) {
    let len = signal.len();
    let mut mdct = Atrac3Mdct::new();
    let mut signal_res = vec![0.0_f32; len];

    let mut enc_bands = [
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
    ];
    let mut dec_bands = [
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
    ];

    let mut reference = Vec::new();

    for pos in (0..len).step_by(WORK_SZ) {
        enc_bands[0][WORK_SZ..WORK_SZ + WORK_SZ].copy_from_slice(&signal[pos..pos + WORK_SZ]);
        let mut specs = vec![0.0_f32; NUM_SAMPLES];

        let mod_curve = (schedule.modulate)(pos);
        let gain: [&[GainPoint]; NUM_QMF] = [&mod_curve, &[], &[], &[]];
        {
            let [b0, b1, b2, b3] = &mut enc_bands;
            let mut enc_refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.mdct_with_gain(&mut specs, &mut enc_refs, &gain);
        }

        if pos == ref_pos {
            reference = specs[..256].to_vec();
        } else if pos >= ref_pos + WORK_SZ && !reference.is_empty() {
            for i in 0..256 {
                assert!(
                    (reference[i] - specs[i]).abs() <= spec_eps,
                    "pos {pos} spec idx {i}: {} != {}, eps {spec_eps}",
                    reference[i],
                    specs[i]
                );
            }
        }

        let (cur, next) = (schedule.demodulate)(pos);
        let demod: [(&[GainPoint], &[GainPoint]); NUM_QMF] =
            [(&cur, &next), (&[], &[]), (&[], &[]), (&[], &[])];
        {
            let [b0, b1, b2, b3] = &mut dec_bands;
            let mut dec_refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.midct_with_gain(&mut specs, &mut dec_refs, &demod);
        }

        signal_res[pos..pos + WORK_SZ].copy_from_slice(&dec_bands[0][..WORK_SZ]);
    }

    (signal_res, reference)
}

#[test]
fn atrac3_mdct_gain2_points_compensate_without_scale_dc() {
    compensate_without_scale(
        &[
            (1024, 1032, RampOp::MulInc),
            (1032, 1048, RampOp::Hold),
            (1048, 1056, RampOp::DivInc),
        ],
        vec![gp(4, 0), gp(1, 3)],
        1.0e-6,
        1.0e-5,
    );
}

#[test]
fn atrac3_mdct_gain2_points_compensate_without_scale_dc2() {
    compensate_without_scale(
        &[
            (1024, 1032, RampOp::MulInc),
            (1032, 1272, RampOp::Hold),
            (1272, 1280, RampOp::DivInc),
        ],
        vec![gp(4, 0), gp(1, 31)],
        1.0e-5,
        1.0e-5,
    );
}

/// Two-frame curves at 1024 and 1024+256 with a scale handoff, mirroring the
/// `*CompensateWithScaleDc*` tests.
fn compensate_with_scale(
    ramp_segments: &[(usize, usize, RampOp)],
    curve0: Vec<GainPoint>,
    curve1: Vec<GainPoint>,
    spec_eps: f32,
    sig_eps: f32,
    scaled_ratio: f32,
) {
    let len = 2048;
    let mut signal = vec![1.0_f32; len];
    apply_ramp(&mut signal, ramp_segments);

    let mut mdct = Atrac3Mdct::new();
    let mut signal_res = vec![0.0_f32; len];

    let mut enc_bands = [
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
    ];
    let mut dec_bands = [
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
    ];

    let mut mdct_result0 = Vec::new();
    let mut mdct_result1 = Vec::new();

    for pos in (0..len).step_by(WORK_SZ) {
        enc_bands[0][WORK_SZ..WORK_SZ + WORK_SZ].copy_from_slice(&signal[pos..pos + WORK_SZ]);
        let mut specs = vec![0.0_f32; NUM_SAMPLES];

        let mod_curve = if pos == 1024 {
            curve0.clone()
        } else if pos == 1024 + 256 {
            curve1.clone()
        } else {
            vec![]
        };
        let gain: [&[GainPoint]; NUM_QMF] = [&mod_curve, &[], &[], &[]];
        {
            let [b0, b1, b2, b3] = &mut enc_bands;
            let mut enc_refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.mdct_with_gain(&mut specs, &mut enc_refs, &gain);
        }

        if pos == 1024 {
            mdct_result0 = specs[..256].to_vec();
        } else if pos == 1024 + 256 {
            mdct_result1 = specs[..256].to_vec();
        } else if pos >= 1024 + 512 && !mdct_result1.is_empty() {
            for i in 0..256 {
                assert!(
                    (mdct_result1[i] - specs[i]).abs() <= spec_eps,
                    "pos {pos} spec idx {i}: {} != {}",
                    mdct_result1[i],
                    specs[i]
                );
            }
        }

        // Demodulation schedule:
        //   1024:       next = curve0
        //   1024+256:   cur = curve0, next = curve1
        //   1024+512:   cur = curve1
        let (cur, next): (Vec<GainPoint>, Vec<GainPoint>) = match pos {
            1024 => (vec![], curve0.clone()),
            x if x == 1024 + 256 => (curve0.clone(), curve1.clone()),
            x if x == 1024 + 512 => (curve1.clone(), vec![]),
            _ => (vec![], vec![]),
        };
        let demod: [(&[GainPoint], &[GainPoint]); NUM_QMF] =
            [(&cur, &next), (&[], &[]), (&[], &[]), (&[], &[])];
        {
            let [b0, b1, b2, b3] = &mut dec_bands;
            let mut dec_refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.midct_with_gain(&mut specs, &mut dec_refs, &demod);
        }

        signal_res[pos..pos + WORK_SZ].copy_from_slice(&dec_bands[0][..WORK_SZ]);
    }

    assert_near_delayed(&signal, &signal_res, sig_eps);

    for i in 0..256 {
        let scaled = mdct_result0[i] / scaled_ratio;
        assert!(
            (scaled - mdct_result1[i]).abs() <= 1.0e-5,
            "scaled spec idx {i}: {scaled} != {}",
            mdct_result1[i]
        );
    }
}

#[test]
fn atrac3_mdct_gain1_point_compensate_with_scale_dc() {
    compensate_with_scale(
        &[
            (1032, 1040, RampOp::MulInc),
            (1040, 1288, RampOp::Hold),
            (1288, 1296, RampOp::DivInc),
        ],
        vec![gp(7, 1)],
        vec![gp(1, 1)],
        1.0e-6,
        1.0e-5,
        8.0,
    );
}

#[test]
fn atrac3_mdct_gain1_point_compensate_with_scale_dc2() {
    compensate_with_scale(
        &[
            (1032, 1040, RampOp::MulInc),
            (1040, 1280, RampOp::Hold),
            (1280, 1288, RampOp::DivInc),
        ],
        vec![gp(7, 1)],
        vec![gp(1, 0)],
        1.0e-6,
        1.0e-5,
        8.0,
    );
}

#[test]
fn atrac3_mdct_gain2_points_compensate_with_scale_dc() {
    compensate_with_scale(
        &[
            (1032, 1040, RampOp::MulInc),
            (1040, 1056, RampOp::Hold),
            (1056, 1064, RampOp::MulInc),
            (1064, 1072, RampOp::Hold),
            (1072, 1080, RampOp::DivInc),
            (1080, 1288, RampOp::Hold),
            (1288, 1296, RampOp::DivInc),
        ],
        vec![gp(7, 1), gp(4, 4), gp(1, 6)],
        vec![gp(1, 1)],
        1.0e-6,
        5.0e-4,
        8.0,
    );
}

/// Port of `TAtrac3MDCTSignalWithGainCompensation`: a sine + DC offset run with
/// a sequence of gain curves across several frames.
#[test]
fn atrac3_mdct_signal_with_gain_compensation() {
    let len = 4096;
    let mut signal = vec![8000.0_f32; len];
    for (i, s) in signal.iter_mut().enumerate().skip(1024) {
        let n = i - 1024;
        *s = 32768.0 * (std::f32::consts::FRAC_PI_2 * n as f32 * 0.25).sin();
    }

    let mut mdct = Atrac3Mdct::new();
    let mut signal_res = vec![0.0_f32; len];

    let mut enc_bands = [
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
    ];
    let mut dec_bands = [
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
        vec![0.0_f32; BAND_BUF_SZ],
    ];

    for pos in (0..len).step_by(WORK_SZ) {
        enc_bands[0][WORK_SZ..WORK_SZ + WORK_SZ].copy_from_slice(&signal[pos..pos + WORK_SZ]);
        let mut specs = vec![0.0_f32; NUM_SAMPLES];

        let mod_curve: Vec<GainPoint> = match pos {
            256 => vec![gp(3, 2)],
            1024 => vec![gp(3, 2), gp(2, 5)],
            x if x == 1024 + 256 => vec![gp(1, 0)],
            2048 => vec![gp(4, 2), gp(1, 5)],
            _ => vec![],
        };
        let gain: [&[GainPoint]; NUM_QMF] = [&mod_curve, &[], &[], &[]];
        {
            let [b0, b1, b2, b3] = &mut enc_bands;
            let mut enc_refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.mdct_with_gain(&mut specs, &mut enc_refs, &gain);
        }

        let (cur, next): (Vec<GainPoint>, Vec<GainPoint>) = match pos {
            256 => (vec![], vec![gp(3, 2)]),
            512 => (vec![gp(3, 2)], vec![]),
            1024 => (vec![], vec![gp(3, 2), gp(2, 5)]),
            x if x == 1024 + 256 => (vec![gp(3, 2), gp(2, 5)], vec![gp(1, 0)]),
            x if x == 1024 + 256 + 256 => (vec![gp(1, 0)], vec![]),
            2048 => (vec![], vec![]),
            x if x == 2048 + 256 => (vec![gp(4, 2), gp(1, 5)], vec![]),
            _ => (vec![], vec![]),
        };
        let demod: [(&[GainPoint], &[GainPoint]); NUM_QMF] =
            [(&cur, &next), (&[], &[]), (&[], &[]), (&[], &[])];
        {
            let [b0, b1, b2, b3] = &mut dec_bands;
            let mut dec_refs = [
                b0.as_mut_slice(),
                b1.as_mut_slice(),
                b2.as_mut_slice(),
                b3.as_mut_slice(),
            ];
            mdct.midct_with_gain(&mut specs, &mut dec_refs, &demod);
        }

        signal_res[pos..pos + WORK_SZ].copy_from_slice(&dec_bands[0][..WORK_SZ]);
    }

    assert_near_delayed(&signal, &signal_res, 0.1);
}
