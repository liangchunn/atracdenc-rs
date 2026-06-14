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
    run_with_rust_log(args, None)
}

fn run_with_rust_log(args: &[&str], rust_log: Option<&str>) -> Output {
    let mut command = Command::new(bin());
    command.args(args);
    if let Some(rust_log) = rust_log {
        command.env("RUST_LOG", rust_log);
    } else {
        command.env_remove("RUST_LOG");
    }
    command.output().unwrap()
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

fn read_be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().unwrap())
}

fn assert_no_temp_for(path: &Path) {
    let Some(dir) = path.parent() else {
        return;
    };
    if !dir.exists() {
        return;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    let prefix = format!(".{file_name}.tmp-");
    let leftovers: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&prefix))
        .collect();
    assert!(
        leftovers.is_empty(),
        "left temp files for {}: {leftovers:?}",
        path.display()
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
    ]);

    assert!(!output.status.success());
    let text = combined_lower(&output);
    assert!(text.contains("unable to open input file"));
    assert!(text.contains("missing.wav"));
    assert!(!text.contains("unsupported sample rate"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn default_encode_emits_info_summary_progress_and_done() {
    let dir = tempdir("default-logs");
    let input = dir.join("in.wav");
    let out = dir.join("out.aea");
    write_wav(&input, 8192);

    let output = run(&[
        "-e",
        "atrac1",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    let text = combined_lower(&output);

    assert_success(output);
    assert!(text.contains("input file:"));
    assert!(text.contains("codec: atrac1"));
    assert!(text.contains("progress: 0% done"));
    assert!(text.contains("progress: 100% done"));
    assert!(text.contains("done"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rust_log_off_suppresses_info_logs() {
    let dir = tempdir("rust-log-off");
    let input = dir.join("in.wav");
    let out = dir.join("out.aea");
    write_wav(&input, 8192);

    let output = run_with_rust_log(
        &[
            "-e",
            "atrac1",
            "-i",
            input.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ],
        Some("off"),
    );
    let text = combined_lower(&output);

    assert_success(output);
    assert!(!text.contains("input file:"));
    assert!(!text.contains("progress:"));
    assert!(!text.contains("codec: atrac1"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rust_log_trace_exposes_facade_and_core_diagnostics() {
    let dir = tempdir("trace-logs");
    let input = dir.join("in.wav");
    let out = dir.join("out.aea");
    write_wav(&input, 8192);

    let output = run_with_rust_log(
        &[
            "-e",
            "atrac1",
            "-i",
            input.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ],
        Some("trace"),
    );
    let text = combined_lower(&output);

    assert_success(output);
    assert!(text.contains("debug atracdenc] validated wav input"));
    assert!(text.contains("trace atracdenc_core::pcm::engine] pcm apply_process start"));
    assert!(text.contains("trace atracdenc_core::at1::codec] atrac1 encode frame"));
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
        ]));
        assert!(fs::metadata(out).unwrap().len() > 0);
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn atrac3_rm_output_uses_computed_frame_count() {
    let dir = tempdir("rm-frame-count");
    let input = dir.join("in.wav");
    let out = dir.join("out.rm");
    write_wav(&input, 4096);

    assert_success(run(&[
        "-e",
        "atrac3",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--notonal",
    ]));

    let bytes = fs::read(&out).unwrap();
    let prop_pos = bytes.windows(4).position(|w| w == b"PROP").unwrap();
    let data_pos = bytes.windows(4).position(|w| w == b"DATA").unwrap();
    assert_eq!(4, read_be_u32(&bytes[prop_pos + 26..prop_pos + 30]));
    assert_eq!(4, read_be_u32(&bytes[data_pos + 10..data_pos + 14]));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn atrac3_yaml_log_path_is_written() {
    let dir = tempdir("yaml-log");
    let input = dir.join("in.wav");
    let out = dir.join("out.oma");
    let yaml_log = dir.join("gain.yaml");
    write_wav(&input, 4096);

    assert_success(run(&[
        "-e",
        "atrac3",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--yaml-log",
        yaml_log.to_str().unwrap(),
        "--notonal",
    ]));
    assert!(fs::metadata(out).unwrap().len() > 0);
    assert!(fs::metadata(yaml_log).unwrap().len() > 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn atrac1_yaml_log_is_rejected_before_touching_bad_yaml_path() {
    let dir = tempdir("at1-yaml-log-rejected");
    let input = dir.join("in.wav");
    let out = dir.join("out.aea");
    let yaml_log = dir.join("missing").join("gain.yaml");
    write_wav(&input, 8192);

    let output = run(&[
        "-e",
        "atrac1",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--yaml-log",
        yaml_log.to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    assert!(combined_lower(&output).contains("yaml-log is only supported for atrac3 encode"));
    assert!(!out.exists());
    assert_no_temp_for(&out);
    assert!(!yaml_log.parent().unwrap().exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn atrac3_bad_yaml_log_path_does_not_create_output_temp() {
    let dir = tempdir("bad-yaml-log");
    let input = dir.join("in.wav");
    let out = dir.join("out.oma");
    let yaml_log = dir.join("missing").join("gain.yaml");
    write_wav(&input, 4096);

    let output = run(&[
        "-e",
        "atrac3",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--yaml-log",
        yaml_log.to_str().unwrap(),
        "--notonal",
    ]);

    assert!(!output.status.success());
    assert!(!out.exists());
    assert_no_temp_for(&out);
    assert!(!yaml_log.parent().unwrap().exists());
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
    ]));
    assert_success(run(&[
        "-d",
        "-i",
        encoded.to_str().unwrap(),
        "-o",
        decoded.to_str().unwrap(),
    ]));
    assert!(fs::metadata(decoded).unwrap().len() > 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn invalid_encode_does_not_truncate_existing_output() {
    let dir = tempdir("invalid-keeps-output");
    let input = dir.join("in.wav");
    let out = dir.join("existing.oma");
    let original = b"existing output bytes";
    write_wav(&input, 2048);
    fs::write(&out, original).unwrap();

    let output = run(&[
        "-e",
        "atrac3",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--bitrate",
        "384",
    ]);

    assert!(!output.status.success());
    assert!(combined_lower(&output).contains("unsupported atrac3 bitrate"));
    assert_eq!(original.as_slice(), fs::read(&out).unwrap());
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
    ]);
    assert!(!invalid_at3p.status.success());
    assert!(combined_lower(&invalid_at3p).contains("container rm is not supported for atrac3plus"));

    let _ = fs::remove_dir_all(dir);
}

fn write_wav_stereo(path: &Path, frames: usize) {
    let mut bytes = Vec::new();
    let data_len = frames as u32 * 2 * 2; // 2 channels * 2 bytes
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes()); // stereo
    bytes.extend_from_slice(&44_100_u32.to_le_bytes());
    bytes.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
    bytes.extend_from_slice(&4_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let l = (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 44_100.0).sin() * 8000.0;
        let r = (2.0 * std::f64::consts::PI * 660.0 * i as f64 / 44_100.0).sin() * 8000.0;
        bytes.extend_from_slice(&(l as i16).to_le_bytes());
        bytes.extend_from_slice(&(r as i16).to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn atrac3plus_oma_smoke() {
    let dir = tempdir("at3p-smoke");
    let input = dir.join("in.wav");
    // 0.5s stereo sine
    write_wav_stereo(&input, 22_050);
    let out = dir.join("out.oma");

    let result = run(&[
        "-e",
        "atrac3plus",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--container",
        "oma",
    ]);
    assert_success(result);

    let bytes = fs::read(&out).unwrap();
    // OMA EA3 header is 96 bytes; AT3+ frames are 2048 bytes each.
    assert!(bytes.len() > 96, "output too small: {}", bytes.len());
    let payload = bytes.len() - 96;
    assert_eq!(payload % 2048, 0, "payload not frame-aligned: {payload}");
    assert!(
        payload / 2048 >= 9,
        "expected ~10 frames, got {}",
        payload / 2048
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn atrac3plus_advanced_opt() {
    let dir = tempdir("at3p-advanced");
    let input = dir.join("in.wav");
    write_wav_stereo(&input, 8192);
    let out = dir.join("out.oma");

    // ghadbg=0 disables GHA passes; should still encode successfully.
    let result = run(&[
        "-e",
        "atrac3plus",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--container",
        "oma",
        "--advanced",
        "ghadbg=0",
    ]);
    assert_success(result);
    assert!(fs::read(&out).unwrap().len() > 96);

    // Invalid mask must fail.
    let bad = run(&[
        "-e",
        "atrac3plus",
        "-i",
        input.to_str().unwrap(),
        "-o",
        dir.join("bad.oma").to_str().unwrap(),
        "--advanced",
        "ghadbg=9",
    ]);
    assert!(!bad.status.success());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn advanced_is_rejected_for_non_atrac3plus() {
    let dir = tempdir("advanced-non-at3p");
    let input = dir.join("in.wav");
    write_wav(&input, 2048);
    let out = dir.join("out.oma");

    let result = run(&[
        "-e",
        "atrac3",
        "-i",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--advanced",
        "ghadbg=0",
    ]);

    assert!(!result.status.success());
    assert!(combined_lower(&result).contains("advanced is only supported for atrac3plus"));
    assert!(!out.exists());
    assert_no_temp_for(&out);

    let decode_out = dir.join("decode.wav");
    let result = run(&[
        "-d",
        "-i",
        input.to_str().unwrap(),
        "-o",
        decode_out.to_str().unwrap(),
        "--advanced",
        "ghadbg=0",
    ]);
    assert!(!result.status.success());
    assert!(combined_lower(&result).contains("advanced is only supported for atrac3plus"));
    assert!(!decode_out.exists());
    assert_no_temp_for(&decode_out);

    let _ = fs::remove_dir_all(dir);
}
