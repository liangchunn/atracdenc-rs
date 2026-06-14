//! Port of `atrac3plus/at3p/at3p_gha_ut.cpp`.
//!
//! Validates GHA tonal extraction: frequency/amplitude indices, stereo
//! leader/sharing decisions, cross-frame tracking, and residual reduction.

use std::f64::consts::PI;

use atracdenc_core::at3p::gha::{At3PGhaData, make_gha_processor0};

#[derive(Clone, Copy)]
struct TestParam {
    freq: f32,
    phase: f32,
    amplitude: u16,
    start: usize,
    end: usize,
}

fn p(freq: f32, phase: f32, amplitude: u16, start: usize, end: usize) -> TestParam {
    TestParam {
        freq,
        phase,
        amplitude,
        start,
        end,
    }
}

fn gen_tone(tp: &TestParam, out: &mut [f32]) {
    let freq = (tp.freq as f64 / (44100.0 / 16.0)) as f32;
    let a = tp.amplitude as f32;
    let mut j = 0i64;
    for i in tp.start..tp.end {
        let fj = freq * (j as f32);
        let arg = fj as f64 * 2.0 * PI + tp.phase as f64;
        out[i] = (out[i] as f64 + arg.sin() * a as f64) as f32;
        j += 1;
    }
}

/// Run one analysis frame; returns the owned result (or default if None).
fn do_analyze(
    proc: &mut dyn atracdenc_core::at3p::gha::GhaProcessor,
    b1: [&[f32]; 2],
    b2: [&[f32]; 2],
) -> Option<At3PGhaData> {
    let mut w1 = [0.0f32; 2048];
    let mut w2 = [0.0f32; 2048];
    proc.do_analyze(b1, b2, &mut w1, &mut w2).cloned()
}

fn gen_and_run(p1: &[TestParam], p2: &[TestParam]) -> At3PGhaData {
    let mut buf1 = vec![0.0f32; 2048 * 2];
    for tp in p1 {
        gen_tone(tp, &mut buf1);
    }
    let stereo = !p2.is_empty();
    let mut buf2 = vec![0.0f32; 2048 * 2];
    for tp in p2 {
        gen_tone(tp, &mut buf2);
    }

    let mut proc = make_gha_processor0(stereo);
    let (b1a, b1b) = buf1.split_at(2048);
    let (b2a, b2b) = buf2.split_at(2048);
    do_analyze(&mut *proc, [b1a, b1b], [b2a, b2b]).expect("tones")
}

// ---- single channel ----

#[test]
fn t689hz0625_full_frame_mono() {
    let res = gen_and_run(&[p(689.0625, 0.0, 32768, 0, 128)], &[]);
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.waves[0].wave_params.len(), 1);
    assert_eq!(res.waves[0].wave_sb_infos.len(), 1);
    assert_eq!(res.num_waves(0, 0), 1);
    let (w, n) = res.waves(0, 0);
    assert_eq!(n, 1);
    assert_eq!(w[0].freq_index, 512);
    assert_eq!(w[0].amp_sf, 63);
}

#[test]
fn t0_full_frame_mono() {
    let res = gen_and_run(&[p(0.0, (PI / 2.0) as f32, 32768, 0, 128)], &[]);
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.waves[0].wave_params.len(), 1);
    assert_eq!(res.num_waves(0, 0), 1);
    let (w, n) = res.waves(0, 0);
    assert_eq!(n, 1);
    assert_eq!(w[0].freq_index, 0);
    assert_eq!(w[0].amp_sf, 63);
}

#[test]
fn t689hz0625_partial_frame_mono() {
    let res = gen_and_run(&[p(689.0625, 0.0, 32768, 32, 128)], &[]);
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.waves[0].wave_params.len(), 1);
    assert_eq!(res.num_waves(0, 0), 1);
    assert_eq!(res.waves(0, 0).0[0].freq_index, 512);
}

#[test]
fn t689hz0625_900hz_full_frame_mono() {
    let res = gen_and_run(
        &[p(689.0625, 0.0, 16384, 0, 128), p(900.0, 0.0, 8192, 0, 128)],
        &[],
    );
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.waves[0].wave_params.len(), 2);
    assert_eq!(res.num_waves(0, 0), 2);
    let (w, n) = res.waves(0, 0);
    assert_eq!(n, 2);
    assert_eq!(w[0].freq_index, 512);
    assert_eq!(w[1].freq_index, 669);
}

#[test]
fn t400hz_800hz_full_frame_mono() {
    let res = gen_and_run(
        &[p(400.0, 0.0, 16384, 0, 128), p(800.0, 0.0, 4096, 0, 128)],
        &[],
    );
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.num_waves(0, 0), 2);
    let (w, _) = res.waves(0, 0);
    assert_eq!(w[0].freq_index, 297);
    assert_eq!(w[1].freq_index, 594);
}

#[test]
fn t689hz0625_2067hz1875_full_frame_mono() {
    let res = gen_and_run(
        &[
            p(689.0625, 0.0, 16384, 0, 128),
            p(689.0625, 0.0, 16384, 128, 256),
        ],
        &[],
    );
    assert_eq!(res.num_tone_bands, 2);
    assert_eq!(res.waves[0].wave_params.len(), 2);
    assert_eq!(res.waves[0].wave_sb_infos.len(), 2);
    assert_eq!(res.num_waves(0, 0), 1);
    assert_eq!(res.num_waves(0, 1), 1);
    assert_eq!(res.waves(0, 0).0[0].freq_index, 512);
    assert_eq!(res.waves(0, 0).0[0].amp_sf, 59);
    assert_eq!(res.waves(0, 1).0[0].freq_index, 512);
    assert_eq!(res.waves(0, 1).0[0].amp_sf, 59);
}

#[test]
fn t689hz0625_4823hz4375_full_frame_mono() {
    let res = gen_and_run(
        &[
            p(689.0625, 0.0, 32768, 0, 128),
            p(689.0625, 0.0, 16384, 256, 384),
        ],
        &[],
    );
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.waves[0].wave_params.len(), 1);
    assert_eq!(res.num_waves(0, 0), 1);
    assert_eq!(res.waves(0, 0).0[0].freq_index, 512);
}

// ---- stereo ----

#[test]
fn t689hz0625_full_frame_stereo_shared() {
    let res = gen_and_run(
        &[p(689.0625, 0.0, 32768, 0, 128)],
        &[p(689.0625, 0.0, 32768, 0, 128)],
    );
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.waves[0].wave_params.len(), 1);
    assert_eq!(res.num_waves(0, 0), 1);
    assert_eq!(res.waves(0, 0).0[0].freq_index, 512);
    assert_eq!(res.tone_sharing[0], true);
    assert_eq!(res.waves[1].wave_params.len(), 0);
}

#[test]
fn t689hz0625_full_frame_stereo_own() {
    let res = gen_and_run(
        &[p(689.0625, 0.0, 32768, 0, 128)],
        &[p(1000.0625, 0.0, 32768, 0, 128)],
    );
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.num_waves(0, 0), 1);
    assert_eq!(res.waves(0, 0).0[0].freq_index, 512);
    assert_eq!(res.tone_sharing[0], false);
    assert_eq!(res.waves[1].wave_params.len(), 1);
    assert_eq!(res.num_waves(1, 0), 1);
    assert_eq!(res.waves(1, 0).0[0].freq_index, 743);
}

#[test]
fn t689hz0625_full_frame_stereo_multiple_second() {
    let res = gen_and_run(
        &[p(689.0625, 0.0, 32768, 0, 128)],
        &[p(689.0625, 0.0, 16384, 0, 128), p(900.0, 0.0, 8192, 0, 128)],
    );
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.num_waves(0, 0), 1);
    assert_eq!(res.waves(0, 0).0[0].freq_index, 512);
    assert_eq!(res.tone_sharing[0], false);
    assert_eq!(res.waves[1].wave_params.len(), 2);
    assert_eq!(res.num_waves(1, 0), 2);
    assert_eq!(res.waves(1, 0).0[0].freq_index, 512);
    assert_eq!(res.waves(1, 0).0[1].freq_index, 669);
}

#[test]
fn t689hz0625_2067hz1875_full_frame_stereo_first_is_leader() {
    let res = gen_and_run(
        &[
            p(689.0625, 0.0, 32768, 0, 128),
            p(689.0625, 0.0, 16384, 128, 256),
        ],
        &[p(689.0625, 0.0, 32768, 0, 128)],
    );
    assert_eq!(res.num_tone_bands, 2);
    assert_eq!(res.waves[0].wave_params.len(), 2);
    assert_eq!(res.num_waves(0, 0), 1);
    assert_eq!(res.num_waves(0, 1), 1);
    assert_eq!(res.waves(0, 0).0[0].freq_index, 512);
    assert_eq!(res.waves(0, 1).0[0].freq_index, 512);
    assert_eq!(res.tone_sharing[0], true);
    assert_eq!(res.tone_sharing[1], false);
    assert_eq!(res.waves[1].wave_params.len(), 0);
    assert_eq!(res.waves[1].wave_sb_infos.len(), 2);
    assert_eq!(res.num_waves(1, 1), 0);
}

#[test]
fn t689hz0625_2067hz1875_full_frame_stereo_second_is_leader() {
    let res = gen_and_run(
        &[p(689.0625, 0.0, 32768, 0, 128)],
        &[
            p(689.0625, 0.0, 32768, 0, 128),
            p(689.0625, 0.0, 16384, 128, 256),
        ],
    );
    assert_eq!(res.num_tone_bands, 2);
    assert_eq!(res.waves[0].wave_params.len(), 2);
    assert_eq!(res.num_waves(0, 0), 1);
    assert_eq!(res.num_waves(0, 1), 1);
    assert_eq!(res.waves(0, 0).0[0].freq_index, 512);
    assert_eq!(res.waves(0, 1).0[0].freq_index, 512);
    assert_eq!(res.tone_sharing[0], true);
    assert_eq!(res.tone_sharing[1], false);
    assert_eq!(res.waves[1].wave_params.len(), 0);
    assert_eq!(res.waves[1].wave_sb_infos.len(), 2);
    assert_eq!(res.num_waves(1, 1), 0);
}

#[test]
fn stereo_sharing_0_2() {
    let res = gen_and_run(
        &[
            p(689.0625, 0.0, 32768, 0, 128),
            p(689.0625, 0.0, 32768, 128, 256),
            p(689.0625, 0.0, 16384, 256, 384),
        ],
        &[
            p(689.0625, 0.0, 32768, 0, 128),
            p(689.0625, 0.0, 16384, 256, 384),
        ],
    );
    assert_eq!(res.num_tone_bands, 3);
    assert_eq!(res.waves[0].wave_params.len(), 3);
    assert_eq!(res.num_waves(0, 0), 1);
    assert_eq!(res.num_waves(0, 1), 1);
    assert_eq!(res.num_waves(0, 2), 1);
    assert_eq!(res.tone_sharing[0], true);
    assert_eq!(res.tone_sharing[1], false);
    assert_eq!(res.tone_sharing[2], true);
    assert_eq!(res.waves[1].wave_params.len(), 0);
    assert_eq!(res.num_waves(1, 1), 0);
}

#[test]
fn stereo_follower_sharing_2() {
    let res = gen_and_run(
        &[
            p(689.0625, 0.0, 32768, 0, 128),
            p(689.0625, 0.0, 32768, 128, 256),
            p(689.0625, 0.0, 16384, 256, 384),
        ],
        &[p(689.0625, 0.0, 16384, 256, 384)],
    );
    assert_eq!(res.num_tone_bands, 3);
    assert_eq!(res.waves[0].wave_params.len(), 3);
    assert_eq!(res.num_waves(0, 0), 1);
    assert_eq!(res.num_waves(0, 1), 1);
    assert_eq!(res.num_waves(0, 2), 1);
    assert_eq!(res.tone_sharing[0], false);
    assert_eq!(res.tone_sharing[1], false);
    assert_eq!(res.tone_sharing[2], true);
    assert_eq!(res.waves[1].wave_params.len(), 0);
    assert_eq!(res.num_waves(1, 0), 0);
    assert_eq!(res.num_waves(1, 1), 0);
}

#[test]
fn stereo_follower_sharing_1() {
    let res = gen_and_run(
        &[
            p(689.0625, 0.0, 32768, 0, 128),
            p(689.0625, 0.0, 32768, 128, 256),
            p(689.0625, 0.0, 16384, 256, 384),
        ],
        &[p(689.0625, 0.0, 16384, 128, 256)],
    );
    assert_eq!(res.num_tone_bands, 3);
    assert_eq!(res.waves[0].wave_params.len(), 3);
    assert_eq!(res.tone_sharing[0], false);
    assert_eq!(res.tone_sharing[1], true);
    assert_eq!(res.tone_sharing[2], false);
    assert_eq!(res.waves[1].wave_params.len(), 0);
    assert_eq!(res.num_waves(1, 0), 0);
    assert_eq!(res.num_waves(1, 2), 0);
}

// ---- multi-frame mono ----

#[test]
fn t100hz_two_frames_mono() {
    let mut buf = vec![0.0f32; 2048 * 2];
    gen_tone(&p(100.0, 0.0, 32768, 0, 256), &mut buf);
    let (a, b) = buf.split_at_mut(2048);
    b[..128].copy_from_slice(&a[128..256]);
    for v in a[128..256].iter_mut() {
        *v = 0.0;
    }

    let mut proc = make_gha_processor0(false);
    {
        let (b1a, b1b) = buf.split_at(2048);
        let res = do_analyze(&mut *proc, [b1a, b1b], [&[], &[]]).unwrap();
        assert_eq!(res.num_tone_bands, 1);
        assert_eq!(res.num_waves(0, 0), 1);
        assert_eq!(res.waves(0, 0).0[0].freq_index, 74);
        assert_eq!(res.waves(0, 0).0[0].amp_sf, 62);
        assert_eq!(res.waves(0, 0).0[0].phase_index, 0);
    }
    {
        for v in buf[0..128].iter_mut() {
            *v = 0.0;
        }
        let (first, second) = buf.split_at(2048);
        // b1 = {&buf[2048], &buf[0]}
        let res = do_analyze(&mut *proc, [second, first], [&[], &[]]).unwrap();
        assert_eq!(res.num_tone_bands, 1);
        assert_eq!(res.num_waves(0, 0), 1);
        assert_eq!(res.waves(0, 0).0[0].freq_index, 74);
        assert_eq!(res.waves(0, 0).0[0].amp_sf, 62);
        assert_eq!(res.waves(0, 0).0[0].phase_index, 21);
    }
}

#[test]
fn t100hz_than_500hz_than_100hz_3_frames_mono() {
    let mut buf = vec![0.0f32; 2048 * 2];
    gen_tone(&p(100.0, 0.0, 32768, 0, 128), &mut buf);
    gen_tone(&p(500.0, 0.0, 32768, 128, 256), &mut buf);
    {
        let (a, b) = buf.split_at_mut(2048);
        b[..128].copy_from_slice(&a[128..256]);
        for v in a[128..256].iter_mut() {
            *v = 0.0;
        }
    }

    let mut proc = make_gha_processor0(false);
    {
        let (b1a, b1b) = buf.split_at(2048);
        let res = do_analyze(&mut *proc, [b1a, b1b], [&[], &[]]).unwrap();
        assert_eq!(res.num_tone_bands, 1);
        assert_eq!(res.waves(0, 0).0[0].freq_index, 74);
        assert_eq!(res.waves(0, 0).0[0].amp_sf, 62);
        assert_eq!(res.waves(0, 0).0[0].phase_index, 0);
    }
    {
        for v in buf[0..128].iter_mut() {
            *v = 0.0;
        }
        gen_tone(&p(100.0, 0.0, 32768, 0, 128), &mut buf);
        let (first, second) = buf.split_at(2048);
        let res = do_analyze(&mut *proc, [second, first], [&[], &[]]).unwrap();
        assert_eq!(res.num_tone_bands, 1);
        assert_eq!(res.waves(0, 0).0[0].freq_index, 372);
        assert_eq!(res.waves(0, 0).0[0].amp_sf, 62);
        assert_eq!(res.waves(0, 0).0[0].phase_index, 0);
    }
    {
        for v in buf[2048..2048 + 128].iter_mut() {
            *v = 0.0;
        }
        let (first, second) = buf.split_at(2048);
        let res = do_analyze(&mut *proc, [first, second], [&[], &[]]).unwrap();
        assert_eq!(res.num_tone_bands, 1);
        assert_eq!(res.waves(0, 0).0[0].freq_index, 74);
        assert_eq!(res.waves(0, 0).0[0].amp_sf, 62);
        assert_eq!(res.waves(0, 0).0[0].phase_index, 0);
    }
}

#[test]
fn t100hz_phase_two_frames_mono() {
    let mut buf = vec![0.0f32; 2048 * 2];
    gen_tone(&p(100.0, (PI * 0.25) as f32, 32768, 0, 256), &mut buf);
    {
        let (a, b) = buf.split_at_mut(2048);
        b[..128].copy_from_slice(&a[128..256]);
        for v in a[128..256].iter_mut() {
            *v = 0.0;
        }
    }
    let mut proc = make_gha_processor0(false);
    let (b1a, b1b) = buf.split_at(2048);
    let res = do_analyze(&mut *proc, [b1a, b1b], [&[], &[]]).unwrap();
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.waves(0, 0).0[0].freq_index, 74);
    assert_eq!(res.waves(0, 0).0[0].amp_sf, 62);
    assert_eq!(res.waves(0, 0).0[0].phase_index, 4);
}

#[test]
fn t689hz0625_two_frames_mono() {
    let mut buf = vec![0.0f32; 2048 * 2];
    gen_tone(&p(689.0625, 0.0, 32768, 0, 256), &mut buf);
    {
        let (a, b) = buf.split_at_mut(2048);
        b[..128].copy_from_slice(&a[128..256]);
        for v in a[128..256].iter_mut() {
            *v = 0.0;
        }
    }
    let mut proc = make_gha_processor0(false);
    let (b1a, b1b) = buf.split_at(2048);
    let res = do_analyze(&mut *proc, [b1a, b1b], [&[], &[]]).unwrap();
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.waves(0, 0).0[0].freq_index, 512);
    assert_eq!(res.waves(0, 0).0[0].amp_sf, 63);
}

#[test]
fn t689hz0625_1000hz_two_frames_mono() {
    let mut buf = vec![0.0f32; 2048 * 2];
    gen_tone(&p(689.0625, 0.0, 16384, 0, 256), &mut buf);
    gen_tone(&p(1000.0, 0.0, 16384, 0, 256), &mut buf);
    {
        let (a, b) = buf.split_at_mut(2048);
        b[..128].copy_from_slice(&a[128..256]);
        for v in a[128..256].iter_mut() {
            *v = 0.0;
        }
    }
    let mut proc = make_gha_processor0(false);
    let (b1a, b1b) = buf.split_at(2048);
    let res = do_analyze(&mut *proc, [b1a, b1b], [&[], &[]]).unwrap();
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.num_waves(0, 0), 2);
    let (w, _) = res.waves(0, 0);
    assert_eq!(w[0].freq_index, 512);
    assert_eq!(w[0].amp_sf, 58);
    assert_eq!(w[1].freq_index, 743);
    assert_eq!(w[1].amp_sf, 58);
}

#[test]
fn t500hz_1000hz_two_frames_mono() {
    let mut buf = vec![0.0f32; 2048 * 2];
    gen_tone(&p(500.0, 0.0, 16384, 0, 256), &mut buf);
    gen_tone(&p(1000.0, 0.0, 2048, 0, 256), &mut buf);
    {
        let (a, b) = buf.split_at_mut(2048);
        b[..128].copy_from_slice(&a[128..256]);
        for v in a[128..256].iter_mut() {
            *v = 0.0;
        }
    }
    let mut proc = make_gha_processor0(false);
    let (b1a, b1b) = buf.split_at(2048);
    let res = do_analyze(&mut *proc, [b1a, b1b], [&[], &[]]).unwrap();
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.num_waves(0, 0), 2);
    let (w, _) = res.waves(0, 0);
    assert_eq!(w[0].freq_index, 372);
    assert_eq!(w[0].amp_sf, 58);
    assert_eq!(w[1].freq_index, 743);
    assert_eq!(w[1].amp_sf, 46);
}

#[test]
fn t500hz_1000hz_phase_two_frames_mono() {
    let mut buf = vec![0.0f32; 2048 * 2];
    gen_tone(&p(500.0, (PI * 0.5) as f32, 16384, 0, 256), &mut buf);
    gen_tone(&p(1000.0, (PI * 0.25) as f32, 2048, 0, 256), &mut buf);
    {
        let (a, b) = buf.split_at_mut(2048);
        b[..128].copy_from_slice(&a[128..256]);
        for v in a[128..256].iter_mut() {
            *v = 0.0;
        }
    }
    let mut proc = make_gha_processor0(false);
    let (b1a, b1b) = buf.split_at(2048);
    let res = do_analyze(&mut *proc, [b1a, b1b], [&[], &[]]).unwrap();
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.num_waves(0, 0), 2);
    let (w, _) = res.waves(0, 0);
    assert_eq!(w[0].freq_index, 372);
    assert_eq!(w[0].amp_sf, 59);
    assert_eq!(w[1].freq_index, 743);
    assert_eq!(w[1].amp_sf, 46);
}

#[test]
fn t250hz_500hz_1000hz_two_frames_mono() {
    let mut buf = vec![0.0f32; 2048 * 2];
    gen_tone(&p(250.0, 0.0, 16384, 0, 256), &mut buf);
    gen_tone(&p(500.0, 0.0, 4096, 0, 256), &mut buf);
    gen_tone(&p(1000.0, 0.0, 2048, 0, 256), &mut buf);
    {
        let (a, b) = buf.split_at_mut(2048);
        b[..128].copy_from_slice(&a[128..256]);
        for v in a[128..256].iter_mut() {
            *v = 0.0;
        }
    }
    let mut proc = make_gha_processor0(false);
    let (b1a, b1b) = buf.split_at(2048);
    let res = do_analyze(&mut *proc, [b1a, b1b], [&[], &[]]).unwrap();
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.num_waves(0, 0), 3);
    let (w, _) = res.waves(0, 0);
    assert_eq!(w[0].freq_index, 186);
    assert_eq!(w[0].amp_sf, 58);
    assert_eq!(w[1].freq_index, 372);
    assert_eq!(w[1].amp_sf, 50);
    assert_eq!(w[2].freq_index, 743);
    assert_eq!(w[2].amp_sf, 46);
}

#[test]
fn t250hz_500hz_1000hz_1200hz_two_frames_mono() {
    let mut buf = vec![0.0f32; 2048 * 2];
    gen_tone(&p(250.0, 0.0, 16384, 0, 256), &mut buf);
    gen_tone(&p(500.0, 0.0, 8000, 0, 256), &mut buf);
    gen_tone(&p(1000.0, 0.0, 4096, 0, 256), &mut buf);
    gen_tone(&p(1200.0, 0.0, 2048, 0, 256), &mut buf);
    {
        let (a, b) = buf.split_at_mut(2048);
        b[..128].copy_from_slice(&a[128..256]);
        for v in a[128..256].iter_mut() {
            *v = 0.0;
        }
    }
    let mut proc = make_gha_processor0(false);
    let (b1a, b1b) = buf.split_at(2048);
    let res = do_analyze(&mut *proc, [b1a, b1b], [&[], &[]]).unwrap();
    assert_eq!(res.num_tone_bands, 1);
    assert_eq!(res.num_waves(0, 0), 4);
    let (w, _) = res.waves(0, 0);
    assert_eq!(w[0].freq_index, 186);
    assert_eq!(w[0].amp_sf, 58);
    assert_eq!(w[1].freq_index, 372);
    assert_eq!(w[1].amp_sf, 54);
    assert_eq!(w[2].freq_index, 743);
    assert_eq!(w[2].amp_sf, 50);
    assert_eq!(w[3].freq_index, 892);
    assert_eq!(w[3].amp_sf, 46);
}

// ---- residual reduction ----

fn check_reduction(f: f32, exp_freq_index: u32) {
    let mut buf = vec![0.0f32; 2048 * 3];
    gen_tone(&p(f, 0.0, 16384, 0, 384), &mut buf);
    {
        let src1: Vec<f32> = buf[128..256].to_vec();
        let src2: Vec<f32> = buf[256..384].to_vec();
        buf[2048..2048 + 128].copy_from_slice(&src1);
        buf[4096..4096 + 128].copy_from_slice(&src2);
        for v in buf[128..384].iter_mut() {
            *v = 0.0;
        }
    }

    let mut proc = make_gha_processor0(false);
    let mut w1 = [0.0f32; 2048];
    let mut w2 = [0.0f32; 2048];
    {
        let (b1a, rest) = buf.split_at(2048);
        let b1b = &rest[..2048];
        let res = proc
            .do_analyze([b1a, b1b], [&[], &[]], &mut w1, &mut w2)
            .expect("tones");
        assert_eq!(res.num_tone_bands, 1);
        assert_eq!(res.waves[0].wave_params.len(), 1);
        assert_eq!(res.waves(0, 0).0[0].freq_index, exp_freq_index);
    }
    {
        w1[..2048].copy_from_slice(&buf[0..2048]);
        let b1a: Vec<f32> = buf[2048..4096].to_vec();
        let b1b: Vec<f32> = buf[4096..6144].to_vec();
        let res = proc
            .do_analyze([&b1a, &b1b], [&[], &[]], &mut w1, &mut w2)
            .expect("tones");
        assert_eq!(res.num_tone_bands, 1);
        assert_eq!(res.waves[0].wave_params.len(), 1);
        assert_eq!(res.waves(0, 0).0[0].freq_index, exp_freq_index);

        let mut e1 = 0.0f64;
        let mut e2 = 0.0f64;
        for i in 0..128 {
            e1 += w1[i] as f64 * w1[i] as f64;
            e2 += buf[i] as f64 * buf[i] as f64;
        }
        let reduction = 5.0 * (e2 / e1).ln();
        assert!(reduction >= 50.0, "reduction {reduction}");
    }
}

#[test]
fn t269hz166_long_frame_mono() {
    check_reduction(269.166, 200);
}

#[test]
fn t999hz948_long_frame_mono() {
    check_reduction(999.948, 743);
}

#[test]
fn t1345hz826_long_frame_mono() {
    check_reduction(1345.826, 1000);
}
