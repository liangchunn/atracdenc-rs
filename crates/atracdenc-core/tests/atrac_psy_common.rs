//! Port of `atracdenc/src/atrac/atrac_psy_common_ut.cpp` (ATRAC1 / ATRAC3
//! cases only; the ATRAC3+ `TScaleTable` cases are out of scope).
//!
//! Covers spectral-flatness behaviour: a uniform block is maximally flat, an
//! impulse maps to a single BFU using the real codec BFU tables, and tonal
//! input is measurably less flat than white noise once routed through each
//! codec's actual MDCT (ATRAC1) or QMF + MDCT (ATRAC3) front-end.

use atracdenc_core::{
    at1::data as at1,
    at3::data as at3,
    atrac::psy::{ENERGY_FLOOR, calc_spectral_flatness_per_bfu},
    dsp::mdct::Mdct,
};

const PI: f32 = std::f32::consts::PI;
const SAMPLE_RATE: f32 = 44_100.0;

/// Codec description needed to drive the generic flatness tests.
#[derive(Clone, Copy)]
struct Codec {
    name: &'static str,
    max_bfus: usize,
    specs_start_long: &'static [u32],
    specs_per_block: &'static [u32],
}

const ATRAC1: Codec = Codec {
    name: "atrac1",
    max_bfus: at1::MAX_BFUS,
    specs_start_long: &at1::SPECS_START_LONG,
    specs_per_block: &at1::SPECS_PER_BLOCK,
};

const ATRAC3: Codec = Codec {
    name: "atrac3",
    max_bfus: at3::MAX_BFUS,
    specs_start_long: &at3::SPECS_START_LONG,
    specs_per_block: &at3::SPECS_PER_BLOCK,
};

impl Codec {
    /// `SpecsStartLong[MaxBfus-1] + SpecsPerBlock[MaxBfus-1]`.
    fn num_specs(&self) -> usize {
        self.specs_start_long[self.max_bfus - 1] as usize
            + self.specs_per_block[self.max_bfus - 1] as usize
    }

    fn flatness(&self, mdct_energy: &[f32]) -> Vec<f32> {
        calc_spectral_flatness_per_bfu(
            mdct_energy,
            &self.specs_start_long[..self.max_bfus],
            &self.specs_per_block[..self.max_bfus],
            self.max_bfus,
            ENERGY_FLOOR,
        )
    }

    fn bfu_energy(&self, mdct_energy: &[f32]) -> Vec<f32> {
        (0..self.max_bfus)
            .map(|bfu| {
                let start = self.specs_start_long[bfu] as usize;
                let len = self.specs_per_block[bfu] as usize;
                mdct_energy[start..start + len].iter().sum()
            })
            .collect()
    }
}

fn calc_sine_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| ((PI * (i as f32 + 0.5)) / n as f32).sin())
        .collect()
}

fn weighted_mean(values: &[f32], weights: &[f32]) -> f32 {
    assert_eq!(values.len(), weights.len());
    let wsum: f32 = weights.iter().sum();
    assert!(wsum > 0.0);
    values
        .iter()
        .zip(weights)
        .map(|(v, w)| v * w)
        .sum::<f32>()
        / wsum
}

/// Deterministic standard-normal-ish noise via Box-Muller. The flatness tests
/// assert margins, not golden values, so an exact std::mt19937 match is not
/// required.
struct Noise {
    state: u64,
}

impl Noise {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn next_uniform(&mut self) -> f32 {
        // SplitMix64.
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        ((z >> 11) as f32) / ((1u64 << 53) as f32)
    }

    fn next_gaussian(&mut self) -> f32 {
        let u1 = self.next_uniform().max(1.0e-12);
        let u2 = self.next_uniform();
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }
}

fn gen_tone(i: usize, tone_hz: f32) -> f32 {
    let phase = 2.0 * PI * tone_hz * i as f32 / SAMPLE_RATE + 0.37;
    phase.sin()
}

fn energy_from_spec(spec: &[f32]) -> Vec<f32> {
    spec.iter().map(|s| 1.0e-12 + s * s).collect()
}

/// ATRAC3 energy: QMF analysis into four sub-bands, each windowed + 512-pt
/// MDCT, mirroring `BuildAtrac3EnergyViaQmfMdct`.
fn build_atrac3_energy(mut sample: impl FnMut(usize) -> f32) -> Vec<f32> {
    use atracdenc_core::at3::{data::ENCODE_WINDOW, qmf::Atrac3AnalysisFilterBank};

    const FRAME_SZ: usize = at3::NUM_SAMPLES; // 1024
    const NUM_FRAMES: usize = 2;
    const SUBBANDS: usize = 4;
    const SUBBAND_SAMPLES: usize = 256;
    const MDCT_INPUT: usize = 512;

    let pcm: Vec<f32> = (0..FRAME_SZ * NUM_FRAMES).map(&mut sample).collect();

    let mut analysis = Atrac3AnalysisFilterBank::new();
    let mut mdct512 = Mdct::new(MDCT_INPUT, 1.0);
    let mut band_state = [[0.0_f32; MDCT_INPUT]; SUBBANDS];
    let mut specs = vec![0.0_f32; at3::NUM_SPECS];

    for frame in 0..NUM_FRAMES {
        let mut subbands = [[0.0_f32; SUBBAND_SAMPLES]; SUBBANDS];
        {
            let [s0, s1, s2, s3] = &mut subbands;
            let mut sub_refs = [
                s0.as_mut_slice(),
                s1.as_mut_slice(),
                s2.as_mut_slice(),
                s3.as_mut_slice(),
            ];
            analysis.analysis(&pcm[frame * FRAME_SZ..(frame + 1) * FRAME_SZ], &mut sub_refs);
        }

        for band in 0..SUBBANDS {
            let state = &mut band_state[band];
            for i in 0..SUBBAND_SAMPLES {
                state[SUBBAND_SAMPLES + i] = subbands[band][i];
            }

            let mut tmp = [0.0_f32; MDCT_INPUT];
            tmp[..SUBBAND_SAMPLES].copy_from_slice(&state[..SUBBAND_SAMPLES]);
            for i in 0..SUBBAND_SAMPLES {
                let cur = state[SUBBAND_SAMPLES + i];
                state[i] = ENCODE_WINDOW[i] * cur;
                tmp[SUBBAND_SAMPLES + i] = ENCODE_WINDOW[SUBBAND_SAMPLES - 1 - i] * cur;
            }

            let spec_band = mdct512.transform(&tmp);
            let dst = &mut specs[band * SUBBAND_SAMPLES..band * SUBBAND_SAMPLES + SUBBAND_SAMPLES];
            dst.copy_from_slice(spec_band);
            if band & 1 != 0 {
                dst.reverse();
            }
        }
    }

    energy_from_spec(&specs)
}

fn build_tone_energy(codec: Codec, tone_hz: f32) -> Vec<f32> {
    match codec.name {
        "atrac1" => {
            let n = 1024;
            let w = calc_sine_window(n);
            let input: Vec<f32> = (0..n).map(|i| gen_tone(i, tone_hz) * w[i]).collect();
            let mut mdct = Mdct::new(n, n as f32);
            energy_from_spec(mdct.transform(&input))
        }
        "atrac3" => build_atrac3_energy(|i| gen_tone(i, tone_hz)),
        other => panic!("unsupported codec {other}"),
    }
}

fn build_white_noise_energy(codec: Codec) -> Vec<f32> {
    let seed = 0x00C0_FFEE_u64 + codec.num_specs() as u64;
    match codec.name {
        "atrac1" => {
            let n = 1024;
            let w = calc_sine_window(n);
            let mut noise = Noise::new(seed);
            let input: Vec<f32> = (0..n).map(|i| noise.next_gaussian() * w[i]).collect();
            let mut mdct = Mdct::new(n, n as f32);
            energy_from_spec(mdct.transform(&input))
        }
        "atrac3" => {
            let mut noise = Noise::new(seed);
            build_atrac3_energy(move |_| noise.next_gaussian())
        }
        other => panic!("unsupported codec {other}"),
    }
}

fn verify_impulse_maps_to_single_bfu(codec: Codec) {
    let num_specs = codec.num_specs();
    let base_energy = vec![1.0_f32; num_specs];

    for bfu in 0..codec.max_bfus {
        let mut mdct_energy = base_energy.clone();
        let impulse_pos = codec.specs_start_long[bfu] as usize;
        mdct_energy[impulse_pos] = 32.0;

        let flatness = codec.flatness(&mdct_energy);
        assert_eq!(flatness.len(), codec.max_bfus);
        assert!(flatness[bfu] < 0.95, "bfu={bfu}");

        for (i, f) in flatness.iter().enumerate() {
            if i != bfu {
                assert!((f - 1.0).abs() < 1.0e-6, "bfu={bfu} changed={i}");
            }
        }
    }
}

fn verify_tone_vs_noise_flatness(codec: Codec) {
    let tone_energy = build_tone_energy(codec, 1000.0);
    let noise_energy = build_white_noise_energy(codec);
    assert_eq!(tone_energy.len(), codec.num_specs());
    assert_eq!(noise_energy.len(), codec.num_specs());

    let tone_flat = codec.flatness(&tone_energy);
    let noise_flat = codec.flatness(&noise_energy);

    let tone_weighted = weighted_mean(&tone_flat, &codec.bfu_energy(&tone_energy));
    let noise_weighted = weighted_mean(&noise_flat, &codec.bfu_energy(&noise_energy));

    assert!(
        noise_weighted > tone_weighted + 0.08,
        "codec={} tone={tone_weighted} noise={noise_weighted}",
        codec.name
    );
}

#[test]
fn spectral_flatness_uniform_block() {
    let flatness = calc_spectral_flatness_per_bfu(&[4.0; 8], &[0], &[8], 1, ENERGY_FLOOR);
    assert_eq!(flatness.len(), 1);
    assert!((flatness[0] - 1.0).abs() < 1.0e-6);
}

#[test]
fn spectral_flatness_bfu_mapping_atrac1() {
    verify_impulse_maps_to_single_bfu(ATRAC1);
}

#[test]
fn spectral_flatness_bfu_mapping_atrac3() {
    verify_impulse_maps_to_single_bfu(ATRAC3);
}

#[test]
fn spectral_flatness_tone_vs_noise_atrac1() {
    verify_tone_vs_noise_flatness(ATRAC1);
}

#[test]
fn spectral_flatness_tone_vs_noise_atrac3() {
    verify_tone_vs_noise_flatness(ATRAC3);
}

#[test]
fn spectral_flatness_10k_tone_atrac3() {
    let tone_energy = build_tone_energy(ATRAC3, 10_000.0);
    let noise_energy = build_white_noise_energy(ATRAC3);

    let tone_flat = ATRAC3.flatness(&tone_energy);
    let noise_flat = ATRAC3.flatness(&noise_energy);

    let tone_weighted = weighted_mean(&tone_flat, &ATRAC3.bfu_energy(&tone_energy));
    let noise_weighted = weighted_mean(&noise_flat, &ATRAC3.bfu_energy(&noise_energy));

    assert!(
        noise_weighted > tone_weighted + 0.05,
        "tone={tone_weighted} noise={noise_weighted}"
    );
}
