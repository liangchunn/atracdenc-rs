//! Integration + regression tests for Sony decode-delay alignment of ATRAC3.
//!
//! These exercise the public `EncodeBuilder` facade against a committed WAV
//! fixture (`tests/fixtures/impulse.wav`, 88_200 stereo samples/channel) and
//! assert the RIFF/AT3 container metadata. Because the repository has no
//! ATRAC3 decoder, correctness of the *audio* (lag 0, faithful length) is
//! validated externally with Sony's `at3tool`; here we lock the framing
//! contract that makes that alignment possible:
//!
//!   * `--sony-delay-align` writes `fact` = true sample count and emits
//!     `ceil((total + 1093) / 1024)` frames (1093 = one frame + the 69-sample
//!     ATRAC3 encoder delay reverse-engineered from `psp_at3tool.exe`).
//!   * Default (non-aligned) encoding is unchanged: `fact` is frame-aligned
//!     and the frame count is `ceil(total / 1024)`. This is the regression
//!     guard that keeps default output byte-compatible with the C++ reference.

use atracdenc::{At3Settings, Codec, Container, EncodeBuilder};

const FIXTURE: &[u8] = include_bytes!("fixtures/impulse.wav");

/// Samples per channel in the fixture (2 s @ 44.1 kHz, deliberately not a
/// multiple of 1024 so the frame-count rounding is meaningful).
const FIXTURE_SAMPLES: u32 = 88_200;
const LP2_FRAME_SIZE: u32 = 384;
const LP4_FRAME_SIZE: u32 = 192;
const ATRAC3_FORMAT_TAG: u16 = 0x0270;
/// One MDCT frame (1024) + Sony's 69-sample ATRAC3 encoder delay.
const DECODER_DELAY: u32 = 1024 + 69;

struct At3Header {
    format_tag: u16,
    fact_samples: u32,
    data_bytes: u32,
}

fn parse_at3(bytes: &[u8]) -> At3Header {
    assert_eq!(b"RIFF", &bytes[0..4], "expected RIFF container");
    assert_eq!(b"WAVE", &bytes[8..12], "expected WAVE form");
    assert_eq!(b"fact", &bytes[52..56], "expected fact chunk at offset 52");
    assert_eq!(b"data", &bytes[68..72], "expected data chunk at offset 68");
    At3Header {
        format_tag: u16::from_le_bytes(bytes[20..22].try_into().unwrap()),
        fact_samples: u32::from_le_bytes(bytes[60..64].try_into().unwrap()),
        data_bytes: u32::from_le_bytes(bytes[72..76].try_into().unwrap()),
    }
}

fn encode(align: bool) -> Vec<u8> {
    encode_codec(Codec::Atrac3, align)
}

fn encode_codec(codec: Codec, align: bool) -> Vec<u8> {
    EncodeBuilder::new()
        .input_bytes(FIXTURE.to_vec())
        .codec(codec)
        .container(Container::Riff)
        .at3_settings(At3Settings {
            sony_delay_align: align,
            ..At3Settings::default()
        })
        .run_to_vec()
        .expect("encode should succeed")
}

#[test]
fn sony_delay_align_writes_true_sample_count_and_flush_frames() {
    let bytes = encode(true);
    let header = parse_at3(&bytes);
    let frames = header.data_bytes / LP2_FRAME_SIZE;
    let expected_frames = (FIXTURE_SAMPLES + DECODER_DELAY).div_ceil(1024);

    assert_eq!(header.format_tag, ATRAC3_FORMAT_TAG);
    assert_eq!(
        header.fact_samples, FIXTURE_SAMPLES,
        "aligned fact must equal the true PCM sample count"
    );
    assert_eq!(
        frames, expected_frames,
        "aligned frame count must include the codec-delay flush frame"
    );
    // For the fixture this is 88 frames vs 87 in default mode.
    assert_eq!(expected_frames, 88);
}

#[test]
fn default_encoding_is_unchanged_regression_guard() {
    let bytes = encode(false);
    let header = parse_at3(&bytes);
    let frames = header.data_bytes / LP2_FRAME_SIZE;
    let expected_frames = FIXTURE_SAMPLES.div_ceil(1024);

    assert_eq!(header.format_tag, ATRAC3_FORMAT_TAG);
    assert_eq!(
        header.fact_samples,
        frames * 1024,
        "default fact stays frame-aligned (C++-compatible)"
    );
    assert_eq!(frames, expected_frames);
    assert_eq!(expected_frames, 87);
}

#[test]
fn alignment_adds_exactly_one_flush_frame_for_this_fixture() {
    let aligned = parse_at3(&encode(true)).data_bytes / LP2_FRAME_SIZE;
    let default = parse_at3(&encode(false)).data_bytes / LP2_FRAME_SIZE;
    assert_eq!(
        aligned,
        default + 1,
        "the codec-delay flush adds one frame so the tail is not clipped"
    );
}

#[test]
fn sony_delay_align_applies_to_lp4_too() {
    // LP4 is the same ATRAC3 codec (same 1024-sample frame and QMF), so the
    // 69-sample encoder delay and 1093-sample flush are identical; only the
    // frame size (192 bytes, joint stereo) differs.
    let header = parse_at3(&encode_codec(Codec::Atrac3Lp4, true));
    let frames = header.data_bytes / LP4_FRAME_SIZE;
    let expected_frames = (FIXTURE_SAMPLES + DECODER_DELAY).div_ceil(1024);

    assert_eq!(header.format_tag, ATRAC3_FORMAT_TAG);
    assert_eq!(header.fact_samples, FIXTURE_SAMPLES);
    assert_eq!(frames, expected_frames);
    assert_eq!(expected_frames, 88);
}
