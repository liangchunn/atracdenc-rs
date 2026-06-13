use std::{
    error::Error,
    fs::{self, File},
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

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
    if opts.yaml_log.is_some() && !matches!(codec, Codec::Atrac3 | Codec::Atrac3Lp4) {
        return Err(invalid_input("--yaml-log is only supported for ATRAC3 encode").into());
    }

    let codec_api = atracdenc::Codec::from(codec);
    let input = opts
        .input
        .ok_or_else(|| invalid_input("missing input file"))?;
    let output = opts
        .output
        .ok_or_else(|| invalid_input("missing output file"))?;
    let input_file = File::open(&input).map_err(|e| {
        invalid_input(format!(
            "unable to open input file {}: {e}",
            input.display()
        ))
    })?;
    let container = opts
        .container
        .map(atracdenc::Container::from)
        .unwrap_or_else(|| infer_container(&output, codec_api));

    let mut builder = atracdenc::EncodeBuilder::new()
        .codec(codec_api)
        .input_reader(BufReader::new(input_file))
        .container(container)
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

    let temp_output = temp_output_path(&output);
    let temp_yaml_log = opts
        .yaml_log
        .as_ref()
        .map(|path| (path, temp_output_path(path)));
    if let Some((_, temp_yaml_log)) = &temp_yaml_log {
        let _ = fs::remove_file(temp_yaml_log);
        let yaml_log_file = File::create(temp_yaml_log)?;
        builder = builder.yaml_log_writer(BufWriter::new(yaml_log_file));
    }

    let _ = fs::remove_file(&temp_output);
    let output_file = match File::create(&temp_output) {
        Ok(file) => file,
        Err(err) => {
            cleanup_temps(&temp_output, &temp_yaml_log);
            return Err(err.into());
        }
    };
    builder = builder.output_writer(BufWriter::new(output_file));

    if let Err(err) = builder.run() {
        cleanup_temps(&temp_output, &temp_yaml_log);
        return Err(err.into());
    }

    replace_output(&temp_output, &output)?;
    if let Some((yaml_log, temp_yaml_log)) = temp_yaml_log {
        replace_output(&temp_yaml_log, yaml_log)?;
    }
    Ok(())
}

fn decode(opts: CliOptions) -> Result<(), Box<dyn Error>> {
    if opts.yaml_log.is_some() {
        return Err(invalid_input("--yaml-log is only supported for ATRAC3 encode").into());
    }

    let codec = opts.encode.unwrap_or(Codec::Atrac1);
    let input = opts
        .input
        .ok_or_else(|| invalid_input("missing input file"))?;
    let output = opts
        .output
        .ok_or_else(|| invalid_input("missing output file"))?;
    let input_file = File::open(&input).map_err(|e| {
        invalid_input(format!(
            "unable to open input file {}: {e}",
            input.display()
        ))
    })?;

    let mut builder = atracdenc::DecodeBuilder::new()
        .codec(codec.into())
        .input_reader(BufReader::new(input_file));
    if let Some(container) = opts.container {
        builder = builder.container(container.into());
    }

    let temp_output = temp_output_path(&output);
    let _ = fs::remove_file(&temp_output);
    let output_file = File::create(&temp_output)?;
    if let Err(err) = builder.output_writer(BufWriter::new(output_file)).run() {
        let _ = fs::remove_file(&temp_output);
        return Err(err.into());
    }
    replace_output(&temp_output, &output)?;
    Ok(())
}

fn invalid_input(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}

fn infer_container(output: &Path, codec: atracdenc::Codec) -> atracdenc::Container {
    let ext = output
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "aea" => atracdenc::Container::Aea,
        "oma" | "omg" => atracdenc::Container::Oma,
        "at3" | "wav" => atracdenc::Container::Riff,
        "rm" | "ra" => atracdenc::Container::Rm,
        "raw" | "dat" => atracdenc::Container::Raw,
        _ => default_container(codec),
    }
}

fn default_container(codec: atracdenc::Codec) -> atracdenc::Container {
    match codec {
        atracdenc::Codec::Atrac1 => atracdenc::Container::Aea,
        atracdenc::Codec::Atrac3 | atracdenc::Codec::Atrac3Lp4 | atracdenc::Codec::Atrac3plus => {
            atracdenc::Container::Oma
        }
    }
}

fn temp_output_path(output: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    output.with_file_name(format!(".{file_name}.tmp-{}-{nanos}", std::process::id()))
}

fn backup_output_path(output: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    output.with_file_name(format!(".{file_name}.bak-{}-{nanos}", std::process::id()))
}

fn cleanup_temps(temp_output: &Path, temp_yaml_log: &Option<(&PathBuf, PathBuf)>) {
    let _ = fs::remove_file(temp_output);
    if let Some((_, temp_yaml_log)) = temp_yaml_log {
        let _ = fs::remove_file(temp_yaml_log);
    }
}

fn replace_output(temp: &Path, output: &Path) -> std::io::Result<()> {
    match fs::rename(temp, output) {
        Ok(()) => Ok(()),
        Err(err) => replace_existing_output(temp, output, err),
    }
}

fn replace_existing_output(
    temp: &Path,
    output: &Path,
    original_err: std::io::Error,
) -> std::io::Result<()> {
    if !output.is_file() {
        return Err(original_err);
    }

    let backup = backup_output_path(output);
    fs::rename(output, &backup)?;

    match fs::rename(temp, output) {
        Ok(()) => {
            fs::remove_file(backup)?;
            Ok(())
        }
        Err(replace_err) => {
            if let Err(restore_err) = fs::rename(&backup, output) {
                return Err(std::io::Error::new(
                    replace_err.kind(),
                    format!(
                        "{replace_err}; failed to restore {} from {}: {restore_err}",
                        output.display(),
                        backup.display()
                    ),
                ));
            }
            Err(replace_err)
        }
    }
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
    fn facade_infers_containers_from_extension() {
        assert_eq!(
            atracdenc::Container::Aea,
            infer_container(Path::new("music.aea"), atracdenc::Codec::Atrac1)
        );
        assert_eq!(
            atracdenc::Container::Oma,
            infer_container(Path::new("music.omg"), atracdenc::Codec::Atrac3)
        );
        assert_eq!(
            atracdenc::Container::Riff,
            infer_container(Path::new("music.at3"), atracdenc::Codec::Atrac3)
        );
        assert_eq!(
            atracdenc::Container::Rm,
            infer_container(Path::new("music.rm"), atracdenc::Codec::Atrac3)
        );
        assert_eq!(
            atracdenc::Container::Oma,
            infer_container(Path::new("music.bin"), atracdenc::Codec::Atrac3)
        );
        assert_eq!(
            atracdenc::Container::Raw,
            infer_container(Path::new("music.dat"), atracdenc::Codec::Atrac3)
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

    fn tempdir(test_name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "atracdenc-cli-main-{test_name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn assert_no_sidecars(dir: &Path, prefix: &str) {
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(prefix))
            .collect();
        assert!(leftovers.is_empty(), "left sidecar files: {leftovers:?}");
    }

    #[test]
    fn replace_existing_output_fallback_swaps_file_and_removes_backup() {
        let dir = tempdir("replace-success");
        let output = dir.join("out.bin");
        let temp = dir.join("temp.bin");
        std::fs::write(&output, b"old").unwrap();
        std::fs::write(&temp, b"new").unwrap();

        replace_existing_output(
            &temp,
            &output,
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, "forced fallback"),
        )
        .unwrap();

        assert_eq!(b"new".as_slice(), std::fs::read(&output).unwrap());
        assert!(!temp.exists());
        assert_no_sidecars(&dir, ".out.bin.bak-");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_existing_output_restores_original_when_temp_is_missing() {
        let dir = tempdir("replace-restore");
        let output = dir.join("out.bin");
        let temp = dir.join("missing-temp.bin");
        std::fs::write(&output, b"old").unwrap();

        let err = replace_existing_output(
            &temp,
            &output,
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, "forced fallback"),
        )
        .unwrap_err();

        assert_eq!(std::io::ErrorKind::NotFound, err.kind());
        assert_eq!(b"old".as_slice(), std::fs::read(&output).unwrap());
        assert_no_sidecars(&dir, ".out.bin.bak-");
        let _ = std::fs::remove_dir_all(dir);
    }
}
