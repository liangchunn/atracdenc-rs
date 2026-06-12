use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_atracdenc")
}

fn tempdir(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "atracdenc-cli-quality-{test_name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_multitone_wav(path: &Path, frames: usize) {
    let mut bytes = Vec::new();
    let data_len = frames as u32 * 2;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&44_100_u32.to_le_bytes());
    bytes.extend_from_slice(&(44_100_u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let t = i as f32 / 44_100.0;
        let v = 0.10 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            + 0.05 * (2.0 * std::f32::consts::PI * 1320.0 * t).sin()
            + 0.03 * (2.0 * std::f32::consts::PI * 3200.0 * t).sin();
        let sample = (v * 32768.0)
            .round()
            .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

fn read_wav_i16_mono(path: &Path) -> Vec<f32> {
    let bytes = fs::read(path).unwrap();
    assert_eq!(b"RIFF", &bytes[0..4]);
    assert_eq!(b"WAVE", &bytes[8..12]);
    assert_eq!(b"data", &bytes[36..40]);
    let data_len = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
    bytes[44..44 + data_len]
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes(chunk.try_into().unwrap()) as f32 / 32768.0)
        .collect()
}

fn run(args: &[&str]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}

fn assert_success(output: Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn snr_for_shift(reference: &[f32], decoded: &[f32], shift: isize) -> Option<(f32, f32)> {
    let (ref_start, dec_start) = if shift >= 0 {
        (shift as usize, 0)
    } else {
        (0, (-shift) as usize)
    };
    let n = reference
        .len()
        .saturating_sub(ref_start)
        .min(decoded.len().saturating_sub(dec_start));
    if n < 4096 {
        return None;
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
    Some((10.0 * (signal / noise).log10(), gain))
}

fn aligned_snr_db(reference: &[f32], decoded: &[f32]) -> (f32, isize, f32) {
    let mut best = (f32::NEG_INFINITY, 0, 1.0);
    for shift in (-4096..=4096).step_by(16) {
        if let Some((snr, gain)) = snr_for_shift(reference, decoded, shift) {
            if snr > best.0 {
                best = (snr, shift, gain);
            }
        }
    }
    let coarse = best.1;
    for shift in coarse - 32..=coarse + 32 {
        if let Some((snr, gain)) = snr_for_shift(reference, decoded, shift) {
            if snr > best.0 {
                best = (snr, shift, gain);
            }
        }
    }
    best
}

#[test]
fn atrac1_cli_encode_decode_keeps_aligned_snr_above_regression_floor() {
    let dir = tempdir("at1-snr");
    let input = dir.join("input.wav");
    let encoded = dir.join("encoded.aea");
    let decoded = dir.join("decoded.wav");
    write_multitone_wav(&input, 48 * 512);

    assert_success(run(&[
        "-e",
        "atrac1",
        "-i",
        input.to_str().unwrap(),
        "-o",
        encoded.to_str().unwrap(),
        "--nostdout",
    ]));
    assert_success(run(&[
        "-d",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        decoded.to_str().unwrap(),
        "--nostdout",
    ]));

    let reference = read_wav_i16_mono(&input);
    let decoded = read_wav_i16_mono(&decoded);
    let (snr, shift, gain) = aligned_snr_db(&reference, &decoded);
    assert!(
        snr >= 8.0,
        "ATRAC1 CLI roundtrip SNR too low: {snr:.2} dB, shift {shift}, gain {gain:.3}"
    );

    let _ = fs::remove_dir_all(dir);
}
