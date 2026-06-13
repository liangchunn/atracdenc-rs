use std::{error::Error, path::PathBuf};

use atracdenc::{At1Settings, At1WindowMode, At3Settings};
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

#[derive(Debug, Args)]
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
    #[arg(long = "bfuidxfast", hide = true)]
    /// not implemented
    bfu_idx_fast: bool,
    #[arg(long = "at3-bfu-mode", value_enum, default_value = "fast")]
    at3_bfu_mode: At3BfuMode,
    #[arg(long = "notransient", num_args = 0..=1, require_equals = true, default_missing_value = "0")]
    no_transient: Option<u32>,
    #[arg(long = "nostdout", hide = true)]
    /// not implemented
    no_stdout: bool,
    #[arg(long = "notonal", alias = "no-tonal-components")]
    no_tonal_components: bool,
    #[arg(long = "nogaincontrol", alias = "no-gain-control")]
    no_gain_control: bool,
    #[arg(long = "yaml-log")]
    yaml_log: Option<PathBuf>,
    #[arg(short = 'm', hide = true)]
    /// not implemented
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

impl From<Codec> for atracdenc::Codec {
    fn from(value: Codec) -> Self {
        match value {
            Codec::Atrac1 => atracdenc::Codec::Atrac1,
            Codec::Atrac3 => atracdenc::Codec::Atrac3,
            Codec::Atrac3Lp4 => atracdenc::Codec::Atrac3Lp4,
            Codec::Atrac3plus => atracdenc::Codec::Atrac3plus,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum Container {
    Aea,
    Oma,
    Riff,
    Rm,
    Raw,
}

impl From<Container> for atracdenc::Container {
    fn from(value: Container) -> Self {
        match value {
            Container::Aea => atracdenc::Container::Aea,
            Container::Oma => atracdenc::Container::Oma,
            Container::Riff => atracdenc::Container::Riff,
            Container::Rm => atracdenc::Container::Rm,
            Container::Raw => atracdenc::Container::Raw,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum At3BfuMode {
    Fast,
    Parity,
}

impl From<At3BfuMode> for atracdenc::At3BfuMode {
    fn from(value: At3BfuMode) -> Self {
        match value {
            At3BfuMode::Fast => atracdenc::At3BfuMode::Fast,
            At3BfuMode::Parity => atracdenc::At3BfuMode::Parity,
        }
    }
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
    let mut builder = atracdenc::EncodeBuilder::new()
        .codec(codec.into())
        .at1_settings(At1Settings {
            bfu_idx_const: opts.bfu_idx_const,
            window_mode: if opts.no_transient.is_some() {
                At1WindowMode::NoTransient
            } else {
                At1WindowMode::Auto
            },
            window_mask: opts.no_transient.unwrap_or(0),
        })
        .at3_settings(At3Settings {
            bitrate_kbps: opts.bitrate,
            no_gain_control: opts.no_gain_control,
            no_tonal_components: opts.no_tonal_components,
            bfu_idx_const: opts.bfu_idx_const,
            bfu_mode: opts.at3_bfu_mode.into(),
            bfu_idx_fast: opts.bfu_idx_fast,
        });

    if let Some(input) = opts.input {
        builder = builder.input_path(input);
    }
    if let Some(output) = opts.output {
        builder = builder.output_path(output);
    }
    if let Some(container) = opts.container {
        builder = builder.container(container.into());
    }
    if let Some(yaml_log) = opts.yaml_log {
        builder = builder.yaml_log_path(yaml_log);
    }

    builder.run()?;
    Ok(())
}

fn decode(opts: CliOptions) -> Result<(), Box<dyn Error>> {
    if opts.yaml_log.is_some() {
        return Err(invalid_input("--yaml-log is only supported for ATRAC3 encode").into());
    }

    let codec = opts.encode.unwrap_or(Codec::Atrac1);
    let mut builder = atracdenc::DecodeBuilder::new().codec(codec.into());
    if let Some(input) = opts.input {
        builder = builder.input_path(input);
    }
    if let Some(output) = opts.output {
        builder = builder.output_path(output);
    }
    if let Some(container) = opts.container {
        builder = builder.container(container.into());
    }

    builder.run()?;
    Ok(())
}

fn invalid_input(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
    fn facade_infers_containers_from_extension() {
        assert_eq!(
            atracdenc::Container::Aea,
            atracdenc::infer_container(Path::new("music.aea"), atracdenc::Codec::Atrac1)
        );
        assert_eq!(
            atracdenc::Container::Oma,
            atracdenc::infer_container(Path::new("music.omg"), atracdenc::Codec::Atrac3)
        );
        assert_eq!(
            atracdenc::Container::Riff,
            atracdenc::infer_container(Path::new("music.at3"), atracdenc::Codec::Atrac3)
        );
        assert_eq!(
            atracdenc::Container::Rm,
            atracdenc::infer_container(Path::new("music.rm"), atracdenc::Codec::Atrac3)
        );
        assert_eq!(
            atracdenc::Container::Oma,
            atracdenc::infer_container(Path::new("music.bin"), atracdenc::Codec::Atrac3)
        );
        assert_eq!(
            atracdenc::Container::Raw,
            atracdenc::infer_container(Path::new("music.dat"), atracdenc::Codec::Atrac3)
        );
    }

    #[test]
    fn at3_bfu_mode_defaults_to_fast() {
        let cli = Cli::try_parse_from(["atracdenc", "-e", "atrac3"]).unwrap();
        assert_eq!(At3BfuMode::Fast, cli.opts.at3_bfu_mode);
    }

    #[test]
    fn rejects_conflicting_at3_bfu_mode_flags() {
        let err = atracdenc::EncodeBuilder::new()
            .codec(atracdenc::Codec::Atrac3)
            .at3_settings(At3Settings {
                bfu_idx_fast: true,
                bfu_mode: atracdenc::At3BfuMode::Parity,
                ..At3Settings::default()
            })
            .run_to_vec()
            .unwrap_err();
        assert_eq!(
            "invalid input: --bfuidxfast cannot be combined with --at3-bfu-mode parity",
            err.to_string()
        );
    }

    #[test]
    fn facade_validates_codec_container_matrix() {
        assert!(
            atracdenc::validate_container(atracdenc::Codec::Atrac1, atracdenc::Container::Aea)
                .is_ok()
        );
        assert!(
            atracdenc::validate_container(atracdenc::Codec::Atrac1, atracdenc::Container::Raw)
                .is_ok()
        );
        assert!(
            atracdenc::validate_container(atracdenc::Codec::Atrac3, atracdenc::Container::Rm)
                .is_ok()
        );
        assert_eq!(
            "invalid input: container oma is not supported for atrac1",
            atracdenc::validate_container(atracdenc::Codec::Atrac1, atracdenc::Container::Oma)
                .unwrap_err()
                .to_string()
        );
        assert_eq!(
            "invalid input: container rm is not supported for atrac3plus",
            atracdenc::validate_container(atracdenc::Codec::Atrac3plus, atracdenc::Container::Rm)
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
