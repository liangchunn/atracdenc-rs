//! Port of `atrac3plus_pqf/ut/ipqf_ut.cpp`.
//!
//! Verifies the analysis PQF against the FFmpeg reference inverse PQF (IPQF),
//! ported here as test scaffolding only. Reference data files are committed
//! under `tests/test_data/at3p_pqf`.

use atracdenc_core::at3p::pqf::{At3pPqf, FF_IPQF_COEFFS1, FF_IPQF_COEFFS2};

const SAMPLES: usize = 8192;
const SUBBANDS: usize = 16;
const SUBBAND_SAMPLES: usize = 128;
const FRAME_SAMPLES: usize = 2048;
const FIR_LEN: usize = 12;

/// fast modulo-23 LUT (FFmpeg `mod23_lut`).
const MOD23_LUT: [usize; 26] = [
    23, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 0,
];

/// Reference inverse PQF channel context (FFmpeg `Atrac3pIPQFChannelCtx`).
struct IpqfCtx {
    buf1: [[f32; 8]; 24],
    buf2: [[f32; 8]; 24],
    pos: usize,
}

impl IpqfCtx {
    fn new() -> Self {
        Self {
            buf1: [[0.0; 8]; 24],
            buf2: [[0.0; 8]; 24],
            pos: 0,
        }
    }
}

/// Naive O(N^2) reference DCT-IV (FFmpeg test `dct4`).
fn dct4_ref(out: &mut [f32], x: &[f32], n: usize, scale: f32) {
    use std::f64::consts::PI;
    for k in 0..n {
        let mut sum = 0.0f64;
        for nn in 0..n {
            sum += x[nn] as f64 * ((PI / n as f64) * (nn as f64 + 0.5) * (k as f64 + 0.5)).cos();
        }
        out[n - 1 - k] = (sum * scale as f64) as f32;
    }
}

/// FFmpeg reference inverse PQF (`ff_atrac3p_ipqf`).
fn ff_atrac3p_ipqf(hist: &mut IpqfCtx, input: &[f32], out: &mut [f32]) {
    for v in out[..FRAME_SAMPLES].iter_mut() {
        *v = 0.0;
    }

    let mut idct_in = [0.0f32; SUBBANDS];
    let mut idct_out = [0.0f32; SUBBANDS];

    for s in 0..SUBBAND_SAMPLES {
        for sb in 0..SUBBANDS {
            idct_in[sb] = input[sb * SUBBAND_SAMPLES + s];
        }

        dct4_ref(&mut idct_out, &idct_in, SUBBANDS, 1.0 / 1024.0);

        for i in 0..8 {
            hist.buf1[hist.pos][i] = idct_out[i + 8];
            hist.buf2[hist.pos][i] = idct_out[7 - i];
        }

        let mut pos_now = hist.pos;
        let mut pos_next = MOD23_LUT[pos_now + 2];

        for t in 0..FIR_LEN {
            for i in 0..8 {
                out[s * 16 + i] += hist.buf1[pos_now][i] * FF_IPQF_COEFFS1[t][i]
                    + hist.buf2[pos_next][i] * FF_IPQF_COEFFS2[t][i];
                out[s * 16 + i + 8] += hist.buf1[pos_now][7 - i] * FF_IPQF_COEFFS1[t][i + 8]
                    + hist.buf2[pos_next][7 - i] * FF_IPQF_COEFFS2[t][i + 8];
            }
            pos_now = MOD23_LUT[pos_next + 2];
            pos_next = MOD23_LUT[pos_now + 2];
        }

        hist.pos = MOD23_LUT[hist.pos];
    }
}

fn test_data_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/test_data/at3p_pqf")
        .join(name)
}

fn read_f32_le(name: &str) -> Vec<f32> {
    let bytes = std::fs::read(test_data_path(name)).expect("read test data");
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn create_chirp(sz: usize, buf: &mut [f32]) {
    use std::f32::consts::PI;
    for i in 0..sz {
        let t = i as f32;
        buf[i] = ((t + t * t * 0.5 / 2.0) * 2.0 * PI / sz as f32).sin();
    }
}

#[test]
fn ipqf_check_on_ref_data() {
    let mr_data = read_f32_le("ipqftest_pcm_mr.dat");
    let ref_data = read_f32_le("ipqftest_pcm_out.dat");
    assert_eq!(mr_data.len(), SAMPLES);
    assert_eq!(ref_data.len(), SAMPLES);

    let mut ctx = IpqfCtx::new();
    let mut tmp = vec![0.0f32; SAMPLES];

    let mut i = 0;
    while i < SAMPLES {
        let mut frame = [0.0f32; FRAME_SAMPLES];
        ff_atrac3p_ipqf(&mut ctx, &mr_data[i..], &mut frame);
        tmp[i..i + FRAME_SAMPLES].copy_from_slice(&frame);
        i += 2048;
    }

    let err = 1.0 / (1u32 << 26) as f32;
    for i in 0..SAMPLES {
        assert!(
            (tmp[i] - ref_data[i]).abs() <= err,
            "i={i} {} != {} (err {err})",
            tmp[i],
            ref_data[i]
        );
    }
}

#[test]
fn ipqf_cmp_energy() {
    let err = 1.0 / (1u64 << 32) as f64;
    let mut e = 0.0f64;
    for i in 0..SUBBANDS {
        let mut e1 = 0.0f64;
        let mut e2 = 0.0f64;
        for j in 0..FIR_LEN {
            e1 += FF_IPQF_COEFFS1[j][i] as f64 * FF_IPQF_COEFFS1[j][i] as f64;
            e2 += FF_IPQF_COEFFS2[j][i] as f64 * FF_IPQF_COEFFS2[j][i] as f64;
        }
        if i != 0 {
            assert!((e - (e1 + e2)).abs() <= err, "band {i}");
        }
        e = e1 + e2;
    }
}

/// Run analyse over `frames` 2048-blocks of `x`, then reference-IPQF back.
fn analyse_then_ipqf(x: &[f32]) -> Vec<f32> {
    let n = x.len();
    assert!(n % 2048 == 0);
    let mut pqf = At3pPqf::new();
    let mut subbands = vec![0.0f32; n];
    for f in 0..(n / 2048) {
        let mut out = [0.0f32; FRAME_SAMPLES];
        pqf.analyse(&x[f * 2048..f * 2048 + 2048], &mut out);
        subbands[f * 2048..f * 2048 + 2048].copy_from_slice(&out);
    }

    let mut sctx = IpqfCtx::new();
    let mut tmp = vec![0.0f32; n];
    for f in 0..(n / 2048) {
        let mut frame = [0.0f32; FRAME_SAMPLES];
        ff_atrac3p_ipqf(&mut sctx, &subbands[f * 2048..], &mut frame);
        tmp[f * 2048..f * 2048 + 2048].copy_from_slice(&frame);
    }
    tmp
}

#[test]
fn pqf_dc_short() {
    let x = [1.0f32; 2048];
    let tmp = analyse_then_ipqf(&x);
    let err = 1.0 / (1u32 << 21) as f32;
    for i in 368..2048 {
        assert!((tmp[i] - x[i]).abs() <= err, "i={i}");
    }
}

#[test]
fn pqf_dc_long() {
    let x = [1.0f32; 4096];
    let tmp = analyse_then_ipqf(&x);
    let err = 1.0 / (1u32 << 21) as f32;
    for i in 368..4096 {
        assert!((tmp[i] - x[i - 368]).abs() <= err, "i={i}");
    }
}

#[test]
fn pqf_seq_short() {
    let mut x = [0.0f32; 2048];
    for i in 0..2048 {
        x[i] = i as f32;
    }
    let tmp = analyse_then_ipqf(&x);
    let err = 2048.0 / (1u32 << 22) as f32;
    for i in 368..2048 {
        assert!((tmp[i] - x[i - 368]).abs() <= err, "i={i}");
    }
}

#[test]
fn pqf_seq_long() {
    let mut x = [0.0f32; 4096];
    for i in 0..4096 {
        x[i] = i as f32;
    }
    let tmp = analyse_then_ipqf(&x);
    let err = 4096.0 / (1u32 << 21) as f32;
    for i in 368..4096 {
        assert!((tmp[i] - x[i - 368]).abs() <= err, "i={i}");
    }
}

#[test]
fn pqf_chirp_short() {
    let mut x = [0.0f32; 2048];
    create_chirp(2048, &mut x);
    let tmp = analyse_then_ipqf(&x);
    let err = 1.0 / (1u32 << 21) as f32;
    for i in 368..2048 {
        assert!((tmp[i] - x[i - 368]).abs() <= err, "i={i}");
    }
}

#[test]
fn pqf_chirp_long() {
    let mut x = [0.0f32; 4096];
    create_chirp(4096, &mut x);
    let tmp = analyse_then_ipqf(&x);
    let err = 1.0 / (1u32 << 21) as f32;
    for i in 368..4096 {
        assert!((tmp[i] - x[i - 368]).abs() <= err, "i={i}");
    }
}

#[test]
fn pqf_noise_long() {
    use rand::prelude::*;
    use rand::rngs::StdRng;
    let mut rng = StdRng::seed_from_u64(0x4154_3350);
    let mut x = [0.0f32; 4096];
    for v in x.iter_mut() {
        *v = rng.random_range(0.0f32..1.0) - 0.5;
    }
    let tmp = analyse_then_ipqf(&x);
    let err = 1.0 / (1u32 << 21) as f32;
    for i in 368..4096 {
        assert!((tmp[i] - x[i - 368]).abs() <= err, "i={i}");
    }
}
