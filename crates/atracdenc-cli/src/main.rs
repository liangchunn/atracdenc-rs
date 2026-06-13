use std::{
    error::Error,
    fs::File,
    io::{self, BufWriter},
    path::{Path, PathBuf},
};

use atracdenc_core::{
    at1::{
        codec::{Atrac1Decoder, Atrac1Encoder},
        data::{EncodeSettings as At1EncodeSettings, WindowMode},
    },
    at3::{
        data::{BfuAllocMode as CoreBfuAllocMode, EncodeSettings as At3EncodeSettings, LP4},
        encoder::Atrac3Encoder,
    },
    container::{
        CompressedInput, CompressedOutput,
        aea::{AEA_FRAME_SIZE, AeaInput, AeaOutput},
        at3::At3Output,
        oma::{OmaChannelFormat, OmaCodec, OmaOutput},
        raw::RawOutput,
        rm::RmOutput,
    },
    pcm::{
        engine::{PcmEngine, PcmEngineError, Processor},
        wav::{WavReader, WavWriter},
    },
};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(author, version, about = "ATRAC encoder")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    opts: CliOptions,
}

#[derive(Debug, Subcommand)]
enum Command {
    Encode(EncodeCommand),
}

#[derive(Debug, Args)]
struct EncodeCommand {
    #[command(flatten)]
    opts: CliOptions,
}

#[derive(Debug, Args, Clone)]
struct CliOptions {
    #[arg(short = 'e', long = "encode", value_enum)]
    encode: Option<Codec>,
    #[arg(short = 'd', long = "decode")]
    decode: bool,
    #[arg(short = 'i', long = "input")]
    input: Option<PathBuf>,
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,
    #[arg(long = "container", value_enum)]
    container: Option<Container>,
    #[arg(long)]
    bitrate: Option<u32>,
    #[arg(long = "bfuidxconst", alias = "bfu-idx-const", default_value_t = 0)]
    bfu_idx_const: u32,
    #[arg(long = "bfuidxfast")]
    bfu_idx_fast: bool,
    #[arg(long = "at3-bfu-mode", value_enum, default_value = "fast")]
    at3_bfu_mode: At3BfuMode,
    #[arg(long = "notransient", num_args = 0..=1, require_equals = true, default_missing_value = "0")]
    no_transient: Option<u32>,
    #[arg(long = "nostdout")]
    no_stdout: bool,
    #[arg(long = "notonal", alias = "no-tonal-components")]
    no_tonal_components: bool,
    #[arg(long = "nogaincontrol", alias = "no-gain-control")]
    no_gain_control: bool,
    #[arg(long = "yaml-log")]
    yaml_log: Option<PathBuf>,
    #[arg(short = 'm')]
    mono: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum Codec {
    Atrac1,
    Atrac3,
    #[value(alias = "atrac3_lp4")]
    Atrac3Lp4,
    Atrac3plus,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum Container {
    Aea,
    Oma,
    Riff,
    Rm,
    Raw,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum At3BfuMode {
    Fast,
    Parity,
}

fn main() {
    if let Err(err) = run(Cli::parse()) {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    let opts = match cli.command {
        Some(Command::Encode(command)) => command.opts,
        None => cli.opts,
    };

    if opts.decode {
        decode(opts)
    } else {
        encode(opts)
    }
}

fn encode(opts: CliOptions) -> Result<(), Box<dyn Error>> {
    let codec = opts.encode.unwrap_or(Codec::Atrac1);
    validate_encode_flags(&opts, codec)?;

    let input = opts
        .input
        .as_ref()
        .ok_or_else(|| invalid_input("missing input file"))?;
    let output = opts
        .output
        .as_ref()
        .ok_or_else(|| invalid_input("missing output file"))?;

    let reader = WavReader::open(input)
        .map_err(|_| invalid_input(format!("unable to open input file {}", input.display())))?;
    let channels = usize::from(reader.channels());
    let sample_rate = reader.sample_rate();

    if channels == 0 || channels > 2 {
        return Err(invalid_input("only mono and stereo WAV input is currently supported").into());
    }
    if sample_rate != 44_100 {
        return Err(invalid_input("unsupported sample rate: only 44100 Hz is supported").into());
    }
    if reader.bits_per_sample() != 16 {
        return Err(invalid_input("unsupported WAV format: only 16-bit PCM is supported").into());
    }
    let frame_count = encoded_frame_count(reader.total_samples(), codec)?;

    let container = opts
        .container
        .unwrap_or_else(|| infer_container(output, codec));
    validate_container(codec, container)?;
    if codec == Codec::Atrac3plus {
        return Err(invalid_input("ATRAC3plus encoding is not ported yet").into());
    }

    let mut processor = build_encoder(&opts, codec, container, channels, sample_rate, frame_count)?;
    let mut engine = PcmEngine::new(
        frame_samples(codec) as u16,
        channels,
        Some(Box::new(reader)),
        None,
    );

    loop {
        match engine.apply_process(frame_samples(codec), processor.as_mut()) {
            Ok(_) => {}
            Err(PcmEngineError::NoDataToRead) => break,
            Err(err) => return Err(invalid_input(format!("PCM processing failed: {err:?}")).into()),
        }
    }

    Ok(())
}

fn decode(opts: CliOptions) -> Result<(), Box<dyn Error>> {
    let codec = opts.encode.unwrap_or(Codec::Atrac1);
    if codec != Codec::Atrac1 {
        return Err(invalid_input("decode is only supported for atrac1").into());
    }
    if opts
        .container
        .is_some_and(|container| container != Container::Aea)
    {
        return Err(invalid_input("decode is only supported from AEA input").into());
    }
    if opts.yaml_log.is_some() {
        return Err(invalid_input("--yaml-log is only supported for ATRAC3 encode").into());
    }

    let input_path = opts
        .input
        .as_ref()
        .ok_or_else(|| invalid_input("missing input file"))?;
    let output_path = opts
        .output
        .as_ref()
        .ok_or_else(|| invalid_input("missing output file"))?;
    let input_file = File::open(input_path).map_err(|_| {
        invalid_input(format!(
            "unable to open input file {}",
            input_path.display()
        ))
    })?;
    let input = AeaInput::new(input_file)?;
    let channels = input.channels().max(1);
    // Match the original atracdenc decode loop: the PCM engine uses a 4096-sample
    // buffer and the driver keeps calling ApplyProcess while the reported audio
    // length has not yet been reached (`while totalSamples > processed`). Because
    // the engine processes whole 4096-sample blocks, the decoder emits the final
    // block in full, rounding the output length up to the next engine block.
    // See atracdenc/src/main.cpp:701 and atracdenc/src/pcmengin.h:152.
    const DECODE_BUFFER_SAMPLES: usize = 4096;
    let total_samples = input.length_in_samples();
    let writer = WavWriter::create(output_path, channels as u16, 44_100)?;
    let mut engine = PcmEngine::new(
        DECODE_BUFFER_SAMPLES as u16,
        channels,
        None,
        Some(Box::new(writer)),
    );
    let mut decoder = Atrac1Decoder::new(Box::new(input));

    let mut processed = 0_u64;
    while total_samples > processed {
        match engine.apply_process(atracdenc_core::at1::data::NUM_SAMPLES, &mut decoder) {
            Ok(p) => processed = p,
            Err(err) => return Err(invalid_input(format!("PCM processing failed: {err:?}")).into()),
        }
    }

    Ok(())
}

fn build_encoder(
    opts: &CliOptions,
    codec: Codec,
    container: Container,
    channels: usize,
    sample_rate: u32,
    frame_count: u32,
) -> Result<Box<dyn Processor>, Box<dyn Error>> {
    match codec {
        Codec::Atrac1 => {
            let output = build_atrac1_output(opts, container, channels, frame_count)?;
            let settings = At1EncodeSettings {
                bfu_idx_const: opts.bfu_idx_const,
                window_mode: if opts.no_transient.is_some() {
                    WindowMode::NoTransient
                } else {
                    WindowMode::Auto
                },
                window_mask: opts.no_transient.unwrap_or(0),
            };
            Ok(Box::new(Atrac1Encoder::new(output, settings)))
        }
        Codec::Atrac3 | Codec::Atrac3Lp4 => {
            let bitrate = match (codec, opts.bitrate) {
                (Codec::Atrac3Lp4, None) => LP4.bitrate,
                (_, bitrate) => bitrate.unwrap_or(0) * 1024,
            };
            let mut settings = At3EncodeSettings::new(
                bitrate,
                opts.no_gain_control,
                opts.no_tonal_components,
                channels as u8,
                opts.bfu_idx_const,
            )
            .ok_or_else(|| invalid_input("unsupported ATRAC3 bitrate"))?;
            settings.bfu_alloc_mode = match opts.at3_bfu_mode {
                At3BfuMode::Fast => CoreBfuAllocMode::Fast,
                At3BfuMode::Parity => CoreBfuAllocMode::Parity,
            };
            let output = build_atrac3_output(
                opts,
                container,
                channels,
                sample_rate,
                frame_count,
                settings,
            )?;
            let yaml_log = opts
                .yaml_log
                .as_ref()
                .map(|path| {
                    File::create(path)
                        .map(|file| Box::new(BufWriter::new(file)) as Box<dyn std::io::Write>)
                })
                .transpose()?;
            Ok(Box::new(Atrac3Encoder::with_yaml_log(
                output, settings, yaml_log,
            )))
        }
        Codec::Atrac3plus => unreachable!("ATRAC3plus is rejected before encoder construction"),
    }
}

fn build_atrac1_output(
    opts: &CliOptions,
    container: Container,
    channels: usize,
    frame_count: u32,
) -> Result<Box<dyn CompressedOutput>, Box<dyn Error>> {
    let output = opts.output.as_ref().expect("output validated");
    let file = BufWriter::new(File::create(output)?);
    match container {
        Container::Aea => Ok(Box::new(AeaOutput::new(
            file,
            "atracdenc-rust",
            channels,
            frame_count,
        )?)),
        Container::Raw => Ok(Box::new(RawOutput::new(
            file,
            channels,
            Some(AEA_FRAME_SIZE),
        ))),
        _ => unreachable!("ATRAC1 container validity checked earlier"),
    }
}

fn build_atrac3_output(
    opts: &CliOptions,
    container: Container,
    channels: usize,
    sample_rate: u32,
    frame_count: u32,
    settings: At3EncodeSettings,
) -> Result<Box<dyn CompressedOutput>, Box<dyn Error>> {
    let output = opts.output.as_ref().expect("output validated");
    let file = BufWriter::new(File::create(output)?);
    let params = settings.container_params;
    match container {
        Container::Oma => {
            let channel_format = if params.joint_stereo {
                OmaChannelFormat::StereoJoint
            } else {
                OmaChannelFormat::Stereo
            };
            Ok(Box::new(OmaOutput::new(
                file,
                OmaCodec::Atrac3,
                sample_rate,
                channel_format,
                params.frame_sz as u32,
            )?))
        }
        Container::Riff => Ok(Box::new(At3Output::atrac3(
            file,
            2,
            frame_count,
            params.frame_sz as u32,
            params.joint_stereo,
        )?)),
        Container::Rm => Ok(Box::new(RmOutput::new(
            file,
            channels,
            frame_count,
            params.frame_sz as u32,
            params.joint_stereo,
        )?)),
        Container::Raw => Ok(Box::new(RawOutput::new(
            file,
            channels,
            Some(params.frame_sz as usize),
        ))),
        _ => unreachable!("ATRAC3 container validity checked earlier"),
    }
}

fn validate_encode_flags(opts: &CliOptions, codec: Codec) -> Result<(), io::Error> {
    if opts.decode {
        return Err(invalid_input(
            "--decode cannot be combined with encode mode",
        ));
    }
    if codec != Codec::Atrac1 && opts.no_transient.is_some() {
        return Err(invalid_input("--notransient is only supported for atrac1"));
    }
    if codec == Codec::Atrac1 && opts.yaml_log.is_some() {
        return Err(invalid_input("--yaml-log is only supported for ATRAC3"));
    }
    if codec == Codec::Atrac1 && (opts.no_gain_control || opts.no_tonal_components) {
        return Err(invalid_input(
            "--nogaincontrol and --notonal are only supported for atrac3",
        ));
    }
    if codec != Codec::Atrac3 && codec != Codec::Atrac3Lp4 && opts.bitrate.is_some() {
        return Err(invalid_input("--bitrate is only supported for atrac3"));
    }
    if let Some(bitrate) = opts.bitrate
        && !(32..=384).contains(&bitrate)
    {
        return Err(invalid_input("--bitrate must be in the range 32..384"));
    }
    if codec == Codec::Atrac1 && opts.bfu_idx_const > 8 {
        return Err(invalid_input(
            "--bfuidxconst must be in the range 0..8 for atrac1",
        ));
    }
    if matches!(codec, Codec::Atrac3 | Codec::Atrac3Lp4)
        && opts.bfu_idx_fast
        && opts.at3_bfu_mode == At3BfuMode::Parity
    {
        return Err(invalid_input(
            "--bfuidxfast cannot be combined with --at3-bfu-mode parity",
        ));
    }
    Ok(())
}

fn validate_container(codec: Codec, container: Container) -> Result<(), io::Error> {
    let supported = match codec {
        Codec::Atrac1 => matches!(container, Container::Aea | Container::Raw),
        Codec::Atrac3 | Codec::Atrac3Lp4 => {
            matches!(
                container,
                Container::Oma | Container::Riff | Container::Rm | Container::Raw
            )
        }
        Codec::Atrac3plus => matches!(container, Container::Oma | Container::Riff | Container::Raw),
    };

    if supported {
        Ok(())
    } else {
        Err(invalid_input(format!(
            "container {} is not supported for {}",
            container_name(container),
            codec_name(codec)
        )))
    }
}

fn infer_container(output: &Path, codec: Codec) -> Container {
    let ext = output
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "aea" => Container::Aea,
        "oma" | "omg" => Container::Oma,
        "at3" | "wav" => Container::Riff,
        "rm" | "ra" => Container::Rm,
        "raw" | "dat" => Container::Raw,
        _ => match codec {
            Codec::Atrac1 => Container::Aea,
            Codec::Atrac3 | Codec::Atrac3Lp4 | Codec::Atrac3plus => Container::Oma,
        },
    }
}

fn encoded_frame_count(total_samples: u64, codec: Codec) -> Result<u32, io::Error> {
    let frames = total_samples.div_ceil(frame_samples(codec) as u64);
    u32::try_from(frames).map_err(|_| invalid_input("input is too long for container metadata"))
}

fn frame_samples(codec: Codec) -> usize {
    match codec {
        Codec::Atrac1 => atracdenc_core::at1::data::NUM_SAMPLES,
        Codec::Atrac3 | Codec::Atrac3Lp4 => atracdenc_core::at3::data::NUM_SAMPLES,
        Codec::Atrac3plus => 2048,
    }
}

fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::Atrac1 => "atrac1",
        Codec::Atrac3 => "atrac3",
        Codec::Atrac3Lp4 => "atrac3_lp4",
        Codec::Atrac3plus => "atrac3plus",
    }
}

fn container_name(container: Container) -> &'static str {
    match container {
        Container::Aea => "aea",
        Container::Oma => "oma",
        Container::Riff => "riff",
        Container::Rm => "rm",
        Container::Raw => "raw",
    }
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_original_style_atrac3_flags() {
        let cli = Cli::try_parse_from([
            "atracdenc",
            "-e",
            "atrac3-lp4",
            "-i",
            "in.wav",
            "-o",
            "out.oma",
            "--nogaincontrol",
            "--notonal",
            "--bfuidxconst",
            "12",
            "--yaml-log",
            "gain.yaml",
            "--at3-bfu-mode",
            "parity",
            "--nostdout",
        ])
        .unwrap();

        assert!(cli.command.is_none());
        assert_eq!(Some(Codec::Atrac3Lp4), cli.opts.encode);
        assert!(cli.opts.no_gain_control);
        assert!(cli.opts.no_tonal_components);
        assert_eq!(12, cli.opts.bfu_idx_const);
        assert_eq!(At3BfuMode::Parity, cli.opts.at3_bfu_mode);
        assert_eq!(Some(PathBuf::from("gain.yaml")), cli.opts.yaml_log);
    }

    #[test]
    fn keeps_encode_subcommand_compatibility() {
        let cli = Cli::try_parse_from([
            "atracdenc",
            "encode",
            "--input",
            "in.wav",
            "--output",
            "out.oma",
            "--encode",
            "atrac3",
        ])
        .unwrap();

        let Some(Command::Encode(command)) = cli.command else {
            panic!("expected encode subcommand");
        };
        assert_eq!(Some(Codec::Atrac3), command.opts.encode);
        assert_eq!(Some(PathBuf::from("in.wav")), command.opts.input);
    }

    #[test]
    fn infers_containers_from_extension() {
        assert_eq!(
            Container::Aea,
            infer_container(Path::new("music.aea"), Codec::Atrac1)
        );
        assert_eq!(
            Container::Oma,
            infer_container(Path::new("music.omg"), Codec::Atrac3)
        );
        assert_eq!(
            Container::Riff,
            infer_container(Path::new("music.at3"), Codec::Atrac3)
        );
        assert_eq!(
            Container::Rm,
            infer_container(Path::new("music.rm"), Codec::Atrac3)
        );
        assert_eq!(
            Container::Oma,
            infer_container(Path::new("music.bin"), Codec::Atrac3)
        );
        assert_eq!(
            Container::Raw,
            infer_container(Path::new("music.dat"), Codec::Atrac3)
        );
    }

    #[test]
    fn validates_atrac1_bfu_idx_const_range() {
        let opts = CliOptions {
            encode: Some(Codec::Atrac1),
            decode: false,
            input: None,
            output: None,
            container: None,
            bitrate: None,
            bfu_idx_const: 9,
            bfu_idx_fast: false,
            at3_bfu_mode: At3BfuMode::Fast,
            no_transient: None,
            no_stdout: true,
            no_tonal_components: false,
            no_gain_control: false,
            yaml_log: None,
            mono: false,
        };

        assert_eq!(
            "--bfuidxconst must be in the range 0..8 for atrac1",
            validate_encode_flags(&opts, Codec::Atrac1)
                .unwrap_err()
                .to_string()
        );
    }

    #[test]
    fn at3_bfu_mode_defaults_to_fast() {
        let cli = Cli::try_parse_from(["atracdenc", "-e", "atrac3"]).unwrap();
        assert_eq!(At3BfuMode::Fast, cli.opts.at3_bfu_mode);
    }

    #[test]
    fn rejects_conflicting_at3_bfu_mode_flags() {
        let cli = Cli::try_parse_from([
            "atracdenc",
            "-e",
            "atrac3",
            "--bfuidxfast",
            "--at3-bfu-mode",
            "parity",
        ])
        .unwrap();

        assert_eq!(
            "--bfuidxfast cannot be combined with --at3-bfu-mode parity",
            validate_encode_flags(&cli.opts, Codec::Atrac3)
                .unwrap_err()
                .to_string()
        );
    }

    #[test]
    fn validates_codec_container_matrix() {
        assert!(validate_container(Codec::Atrac1, Container::Aea).is_ok());
        assert!(validate_container(Codec::Atrac1, Container::Raw).is_ok());
        assert!(validate_container(Codec::Atrac3, Container::Rm).is_ok());
        assert_eq!(
            "container oma is not supported for atrac1",
            validate_container(Codec::Atrac1, Container::Oma)
                .unwrap_err()
                .to_string()
        );
        assert_eq!(
            "container rm is not supported for atrac3plus",
            validate_container(Codec::Atrac3plus, Container::Rm)
                .unwrap_err()
                .to_string()
        );
    }

    #[test]
    fn parses_notransient_optional_mask() {
        let cli = Cli::try_parse_from(["atracdenc", "--notransient=5"]).unwrap();
        assert_eq!(Some(5), cli.opts.no_transient);

        let cli = Cli::try_parse_from(["atracdenc", "--notransient"]).unwrap();
        assert_eq!(Some(0), cli.opts.no_transient);
    }
}
