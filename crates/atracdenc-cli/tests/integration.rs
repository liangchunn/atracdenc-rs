use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

const UTF8_STEM: &str = "é-入力-тест";

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_atracdenc")
}

fn tempdir(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "atracdenc-cli-{test_name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_wav(path: &Path, frames: usize) {
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
        let phase = 2.0 * std::f64::consts::PI * 440.0 * i as f64 / 44_100.0;
        let sample = (phase.sin() * 8000.0) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

fn run(args: &[&str]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}

fn combined_lower(output: &Output) -> String {
    let mut bytes = output.stdout.clone();
    bytes.extend_from_slice(&output.stderr);
    String::from_utf8_lossy(&bytes).to_lowercase()
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

#[test]
fn missing_input_reports_open_error_before_wav_validation() {
    let dir = tempdir("missing-input");
    let missing = dir.join("missing.wav");
    let out = dir.join("out.oma");

    let output = run(&[
        "-e",
        "atrac3",
        "-i",
        missing.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--nostdout",
    ]);

    assert!(!output.status.success());
    let text = combined_lower(&output);
    assert!(text.contains("unable to open input file"));
    assert!(text.contains("missing.wav"));
    assert!(!text.contains("unsupported sample rate"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn utf8_input_atrac3_encode_succeeds() {
    let dir = tempdir("utf8-input-at3");
    let input = dir.join(format!("{UTF8_STEM}.wav"));
    let out = dir.join("out.oma");
    write_wav(&input, 2048);

    assert_success(run(&[
        "-e",
        "atrac3",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--notonal",
        "--nostdout",
    ]));
    assert!(fs::metadata(out).unwrap().len() > 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn utf8_input_atrac1_encode_succeeds() {
    let dir = tempdir("utf8-input-at1");
    let input = dir.join(format!("{UTF8_STEM}.wav"));
    let out = dir.join("out.aea");
    write_wav(&input, 8192);

    assert_success(run(&[
        "-e",
        "atrac1",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--nostdout",
    ]));
    assert!(fs::metadata(out).unwrap().len() > 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn utf8_atrac3_output_extensions_succeed() {
    let dir = tempdir("utf8-at3-outputs");
    let input = dir.join("in.wav");
    write_wav(&input, 4096);

    for ext in ["oma", "at3", "rm"] {
        let out = dir.join(format!("{UTF8_STEM}.{ext}"));
        assert_success(run(&[
            "-e",
            "atrac3",
            "-i",
            input.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--notonal",
            "--nostdout",
        ]));
        assert!(fs::metadata(out).unwrap().len() > 0);
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn utf8_atrac1_output_and_decode_succeed() {
    let dir = tempdir("utf8-at1-decode");
    let input = dir.join("in.wav");
    let encoded = dir.join(format!("{UTF8_STEM}.aea"));
    let decoded = dir.join(format!("{UTF8_STEM}.wav"));
    write_wav(&input, 8192);

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
    assert!(fs::metadata(decoded).unwrap().len() > 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn explicit_container_overrides_extension_and_invalid_combos_fail() {
    let dir = tempdir("explicit-container");
    let input = dir.join("in.wav");
    write_wav(&input, 4096);

    let riff_named_oma = dir.join("forced-riff.oma");
    assert_success(run(&[
        "-e",
        "atrac3",
        "-i",
        input.to_str().unwrap(),
        "-o",
        riff_named_oma.to_str().unwrap(),
        "--container",
        "riff",
        "--notonal",
        "--nostdout",
    ]));
    assert_eq!(b"RIFF", &fs::read(&riff_named_oma).unwrap()[..4]);

    let raw_named_aea = dir.join("forced-raw.aea");
    assert_success(run(&[
        "-e",
        "atrac1",
        "-i",
        input.to_str().unwrap(),
        "-o",
        raw_named_aea.to_str().unwrap(),
        "--container",
        "raw",
        "--nostdout",
    ]));
    assert!(fs::metadata(raw_named_aea).unwrap().len() > 0);

    let invalid_at1 = run(&[
        "-e",
        "atrac1",
        "-i",
        input.to_str().unwrap(),
        "-o",
        dir.join("bad.oma").to_str().unwrap(),
        "--container",
        "oma",
        "--nostdout",
    ]);
    assert!(!invalid_at1.status.success());
    assert!(combined_lower(&invalid_at1).contains("container oma is not supported for atrac1"));

    let invalid_at3p = run(&[
        "-e",
        "atrac3plus",
        "-i",
        input.to_str().unwrap(),
        "-o",
        dir.join("bad.rm").to_str().unwrap(),
        "--container",
        "rm",
        "--nostdout",
    ]);
    assert!(!invalid_at3p.status.success());
    assert!(combined_lower(&invalid_at3p).contains("container rm is not supported for atrac3plus"));

    let _ = fs::remove_dir_all(dir);
}
