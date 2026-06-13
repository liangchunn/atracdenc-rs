use std::{fs, io::Cursor, path::PathBuf};

use atracdenc_core::{
    at1::{
        codec::{Atrac1Decoder, Atrac1Encoder},
        data::{EncodeSettings, NUM_SAMPLES},
    },
    container::aea::{AEA_FRAME_SIZE, AEA_META_SIZE, AeaInput, AeaOutput},
    pcm::engine::{ProcessMeta, ProcessResult, Processor},
};

fn multitone(frames: usize) -> Vec<f32> {
    let mut pcm = vec![0.0_f32; frames * NUM_SAMPLES];
    for (i, sample) in pcm.iter_mut().enumerate() {
        let t = i as f32 / 44_100.0;
        *sample = 0.10 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            + 0.05 * (2.0 * std::f32::consts::PI * 1320.0 * t).sin()
            + 0.03 * (2.0 * std::f32::consts::PI * 3200.0 * t).sin();
    }
    pcm
}

fn temp_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("atracdenc-{name}-{}.aea", std::process::id()));
    path
}

fn encode_aea_mono(pcm: &[f32]) -> Vec<u8> {
    let path = temp_path("at1-quality");
    {
        let file = fs::File::create(&path).unwrap();
        let output =
            AeaOutput::new(file, "roundtrip", 1, (pcm.len() / NUM_SAMPLES) as u32).unwrap();
        let mut encoder = Atrac1Encoder::new(Box::new(output), EncodeSettings::default());

        for chunk in pcm.chunks_exact(NUM_SAMPLES) {
            let mut frame = chunk.to_vec();
            assert_eq!(
                ProcessResult::Processed,
                encoder
                    .process_frame(&mut frame, &ProcessMeta { channels: 1 })
                    .unwrap()
            );
        }
    }

    let bytes = fs::read(&path).unwrap();
    let _ = fs::remove_file(path);
    bytes
}

fn decode_aea_mono(bytes: Vec<u8>) -> Vec<f32> {
    let num_frames = bytes
        .len()
        .saturating_sub(AEA_META_SIZE)
        .checked_div(AEA_FRAME_SIZE)
        .unwrap_or(0);
    let input = AeaInput::new(Cursor::new(bytes)).unwrap();
    let mut decoder = Atrac1Decoder::new(Box::new(input));
    let mut decoded = Vec::with_capacity(num_frames * NUM_SAMPLES);
    for _ in 0..num_frames {
        let mut frame = vec![0.0; NUM_SAMPLES];
        assert_eq!(
            ProcessResult::Processed,
            decoder
                .process_frame(&mut frame, &ProcessMeta { channels: 1 })
                .unwrap()
        );
        decoded.extend(frame);
    }
    decoded
}

fn aligned_snr_db(reference: &[f32], decoded: &[f32], max_shift: isize) -> (f32, isize, f32) {
    let mut best = (f32::NEG_INFINITY, 0, 1.0);
    for shift in -max_shift..=max_shift {
        let (ref_start, dec_start) = if shift >= 0 {
            (shift as usize, 0)
        } else {
            (0, (-shift) as usize)
        };
        let n = (reference.len().saturating_sub(ref_start))
            .min(decoded.len().saturating_sub(dec_start));
        if n < 4096 {
            continue;
        }
        let trim = 512.min(n / 8);
        let n = n - trim * 2;
        let ref_slice = &reference[ref_start + trim..ref_start + trim + n];
        let dec_slice = &decoded[dec_start + trim..dec_start + trim + n];

        let dot = ref_slice
            .iter()
            .zip(dec_slice)
            .map(|(x, y)| x * y)
            .sum::<f32>();
        let ref_energy = ref_slice.iter().map(|x| x * x).sum::<f32>().max(1.0e-20);
        let gain = dot / ref_energy;
        let signal = ref_slice
            .iter()
            .map(|x| {
                let v = gain * x;
                v * v
            })
            .sum::<f32>();
        let noise = ref_slice
            .iter()
            .zip(dec_slice)
            .map(|(x, y)| {
                let e = gain * x - y;
                e * e
            })
            .sum::<f32>()
            .max(1.0e-20);
        let snr = 10.0 * (signal / noise).log10();
        if snr > best.0 {
            best = (snr, shift, gain);
        }
    }
    best
}

#[test]
fn atrac1_mono_aea_roundtrip_keeps_aligned_snr_above_regression_floor() {
    let reference = multitone(48);
    let encoded = encode_aea_mono(&reference);
    assert_eq!(0, (encoded.len() - AEA_META_SIZE) % AEA_FRAME_SIZE);

    let decoded = decode_aea_mono(encoded);
    assert!(decoded.iter().all(|x| x.is_finite()));

    let (snr, shift, gain) = aligned_snr_db(&reference, &decoded, 4096);
    // Current Rust encoder/decoder measures roughly 10 dB on this deterministic
    // multitone after ATRAC1's preroll/delay and gain alignment. This is a
    // regression floor, not the final C++-calibrated acceptance threshold.
    assert!(
        snr >= 8.0,
        "ATRAC1 roundtrip SNR too low: {snr:.2} dB, shift {shift}, gain {gain:.3}"
    );
}
