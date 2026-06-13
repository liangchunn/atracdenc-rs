use std::{
    cell::RefCell,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    rc::Rc,
};

use atracdenc_core::{
    AtracdencError,
    at1::{
        codec::{Atrac1Decoder, Atrac1Encoder},
        data::{EncodeSettings as CoreAt1Settings, WindowMode},
    },
    at3::{
        data::{BfuAllocMode as CoreBfuAllocMode, EncodeSettings as CoreAt3Settings, LP4},
        encoder::Atrac3Encoder,
    },
    container::{
        CompressedInput, CompressedOutput, ContainerError,
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

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error(transparent)]
    Core(#[from] AtracdencError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<ContainerError> for Error {
    fn from(value: ContainerError) -> Self {
        Error::Core(AtracdencError::from(value))
    }
}

trait ReadSeek: Read + Seek {}

impl<T: Read + Seek> ReadSeek for T {}

trait WriteSeek: Write + Seek {}

impl<T: Write + Seek> WriteSeek for T {}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Codec {
    Atrac1,
    Atrac3,
    Atrac3Lp4,
    Atrac3plus,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Container {
    Aea,
    Oma,
    Riff,
    Rm,
    Raw,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum At3BfuMode {
    Fast,
    Parity,
}

impl From<At3BfuMode> for CoreBfuAllocMode {
    fn from(value: At3BfuMode) -> Self {
        match value {
            At3BfuMode::Fast => CoreBfuAllocMode::Fast,
            At3BfuMode::Parity => CoreBfuAllocMode::Parity,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct At1Settings {
    pub bfu_idx_const: u32,
    pub window_mode: At1WindowMode,
    pub window_mask: u32,
}

impl Default for At1Settings {
    fn default() -> Self {
        Self {
            bfu_idx_const: 0,
            window_mode: At1WindowMode::Auto,
            window_mask: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum At1WindowMode {
    Auto,
    NoTransient,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct At3Settings {
    pub bitrate_kbps: Option<u32>,
    pub no_gain_control: bool,
    pub no_tonal_components: bool,
    pub bfu_idx_const: u32,
    pub bfu_mode: At3BfuMode,
    pub bfu_idx_fast: bool,
}

impl Default for At3Settings {
    fn default() -> Self {
        Self {
            bitrate_kbps: None,
            no_gain_control: false,
            no_tonal_components: false,
            bfu_idx_const: 0,
            bfu_mode: At3BfuMode::Fast,
            bfu_idx_fast: false,
        }
    }
}

pub struct EncodeBuilder {
    input: Option<EncodeInput>,
    output: Option<EncodeOutput>,
    codec: Codec,
    container: Option<Container>,
    at1: At1Settings,
    at3: At3Settings,
    yaml_log: Option<Box<dyn Write>>,
}

enum EncodeInput {
    Reader(Box<dyn Read>),
}

enum EncodeOutput {
    Writer(Box<dyn WriteSeek>),
    Vec,
}

impl Default for EncodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl EncodeBuilder {
    pub fn new() -> Self {
        Self {
            input: None,
            output: None,
            codec: Codec::Atrac1,
            container: None,
            at1: At1Settings::default(),
            at3: At3Settings::default(),
            yaml_log: None,
        }
    }

    pub fn input_reader<R: Read + 'static>(mut self, reader: R) -> Self {
        self.input = Some(EncodeInput::Reader(Box::new(reader)));
        self
    }

    pub fn input_bytes(self, bytes: impl Into<Vec<u8>>) -> Self {
        self.input_reader(Cursor::new(bytes.into()))
    }

    pub fn output_writer<W: Write + Seek + 'static>(mut self, writer: W) -> Self {
        self.output = Some(EncodeOutput::Writer(Box::new(writer)));
        self
    }

    pub fn codec(mut self, codec: Codec) -> Self {
        self.codec = codec;
        self
    }

    pub fn container(mut self, container: Container) -> Self {
        self.container = Some(container);
        self
    }

    pub fn at1_settings(mut self, settings: At1Settings) -> Self {
        self.at1 = settings;
        self
    }

    pub fn at3_settings(mut self, settings: At3Settings) -> Self {
        self.at3 = settings;
        self
    }

    pub fn yaml_log_writer<W: Write + 'static>(mut self, writer: W) -> Self {
        self.yaml_log = Some(Box::new(writer));
        self
    }

    pub fn run(self) -> Result<()> {
        encode(self).map(|_| ())
    }

    pub fn run_to_vec(mut self) -> Result<Vec<u8>> {
        self.output = Some(EncodeOutput::Vec);
        encode(self).and_then(|output| match output {
            EncodedOutput::Vec(bytes) => Ok(bytes),
            EncodedOutput::Sink => Err(invalid_input("internal error: expected byte output")),
        })
    }
}

pub struct DecodeBuilder {
    input: Option<DecodeInput>,
    output: Option<DecodeOutput>,
    codec: Codec,
    container: Option<Container>,
}

enum DecodeInput {
    Reader(Box<dyn ReadSeek>),
}

enum DecodeOutput {
    Writer(Box<dyn WriteSeek>),
    Vec,
}

impl Default for DecodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeBuilder {
    pub fn new() -> Self {
        Self {
            input: None,
            output: None,
            codec: Codec::Atrac1,
            container: None,
        }
    }

    pub fn input_reader<R: Read + Seek + 'static>(mut self, reader: R) -> Self {
        self.input = Some(DecodeInput::Reader(Box::new(reader)));
        self
    }

    pub fn input_bytes(self, bytes: impl Into<Vec<u8>>) -> Self {
        self.input_reader(Cursor::new(bytes.into()))
    }

    pub fn output_writer<W: Write + Seek + 'static>(mut self, writer: W) -> Self {
        self.output = Some(DecodeOutput::Writer(Box::new(writer)));
        self
    }

    pub fn codec(mut self, codec: Codec) -> Self {
        self.codec = codec;
        self
    }

    pub fn container(mut self, container: Container) -> Self {
        self.container = Some(container);
        self
    }

    pub fn run(self) -> Result<()> {
        decode(self).map(|_| ())
    }

    pub fn run_to_vec(mut self) -> Result<Vec<u8>> {
        self.output = Some(DecodeOutput::Vec);
        decode(self).and_then(|output| match output {
            DecodedOutput::Vec(bytes) => Ok(bytes),
            DecodedOutput::Sink => Err(invalid_input("internal error: expected byte output")),
        })
    }
}

enum EncodedOutput {
    Sink,
    Vec(Vec<u8>),
}

enum DecodedOutput {
    Sink,
    Vec(Vec<u8>),
}

fn encode(mut builder: EncodeBuilder) -> Result<EncodedOutput> {
    validate_encode_settings(&builder)?;
    let input = builder
        .input
        .take()
        .ok_or_else(|| invalid_input("missing input file"))?;
    let reader = open_wav_reader(input)?;
    let channels = usize::from(reader.channels());
    let sample_rate = reader.sample_rate();

    validate_wav(&reader)?;
    let frame_count = encoded_frame_count(reader.total_samples(), builder.codec)?;

    let output = builder
        .output
        .take()
        .ok_or_else(|| invalid_input("missing output file"))?;
    let container = match (builder.container, &output) {
        (Some(container), _) => container,
        (None, EncodeOutput::Writer(_) | EncodeOutput::Vec) => default_container(builder.codec),
    };
    validate_container(builder.codec, container)?;
    if builder.codec == Codec::Atrac3plus {
        return Err(invalid_input("ATRAC3plus encoding is not ported yet"));
    }

    let mut vec_output = None;
    let mut processor = match output {
        EncodeOutput::Writer(writer) => build_seek_encoder(
            &mut builder,
            writer,
            container,
            channels,
            sample_rate,
            frame_count,
        )?,
        EncodeOutput::Vec => {
            let shared = SharedCursor::default();
            vec_output = Some(shared.clone());
            build_seek_encoder(
                &mut builder,
                shared,
                container,
                channels,
                sample_rate,
                frame_count,
            )?
        }
    };
    let mut engine = PcmEngine::new(
        frame_samples(builder.codec),
        channels,
        Some(Box::new(reader)),
        None,
    );

    loop {
        match engine.apply_process(frame_samples(builder.codec), processor.as_mut()) {
            Ok(_) => {}
            Err(AtracdencError::PcmEngine(PcmEngineError::NoDataToRead)) => break,
            Err(err) => return Err(err.into()),
        }
    }
    drop(processor);

    if let Some(output) = vec_output {
        Ok(EncodedOutput::Vec(output.into_bytes()))
    } else {
        Ok(EncodedOutput::Sink)
    }
}

fn decode(mut builder: DecodeBuilder) -> Result<DecodedOutput> {
    if builder.codec != Codec::Atrac1 {
        return Err(invalid_input("decode is only supported for atrac1"));
    }
    if builder
        .container
        .is_some_and(|container| container != Container::Aea)
    {
        return Err(invalid_input("decode is only supported from AEA input"));
    }

    let input = builder
        .input
        .take()
        .ok_or_else(|| invalid_input("missing input file"))?;
    let output = builder
        .output
        .take()
        .ok_or_else(|| invalid_input("missing output file"))?;
    let input = open_aea_input(input)?;
    let channels = input.channels().max(1);
    const DECODE_BUFFER_SAMPLES: usize = 4096;
    let total_samples = input.length_in_samples();

    let mut vec_output = None;
    let writer: Box<dyn atracdenc_core::pcm::engine::PcmWriter> = match output {
        DecodeOutput::Writer(writer) => Box::new(
            WavWriter::new(writer, channels as u16, 44_100)
                .map_err(|e| invalid_input(e.to_string()))?,
        ),
        DecodeOutput::Vec => {
            let shared = SharedCursor::default();
            vec_output = Some(shared.clone());
            Box::new(
                WavWriter::new(shared, channels as u16, 44_100)
                    .map_err(|e| invalid_input(e.to_string()))?,
            )
        }
    };
    let mut engine = PcmEngine::new(DECODE_BUFFER_SAMPLES, channels, None, Some(writer));
    let mut decoder = Atrac1Decoder::new(Box::new(input));

    let mut processed = 0_u64;
    while total_samples > processed {
        match engine.apply_process(atracdenc_core::at1::data::NUM_SAMPLES, &mut decoder) {
            Ok(p) => processed = p,
            Err(err) => return Err(err.into()),
        }
    }

    drop(engine);

    if let Some(output) = vec_output {
        Ok(DecodedOutput::Vec(output.into_bytes()))
    } else {
        Ok(DecodedOutput::Sink)
    }
}

fn open_wav_reader(input: EncodeInput) -> Result<WavReader<Box<dyn Read>>> {
    match input {
        EncodeInput::Reader(reader) => WavReader::new(reader)
            .map_err(|e| invalid_input(format!("unable to read WAV input: {e}"))),
    }
}

fn open_aea_input(input: DecodeInput) -> Result<AeaInput<Box<dyn ReadSeek>>> {
    match input {
        DecodeInput::Reader(reader) => Ok(AeaInput::new(reader)?),
    }
}

fn build_seek_encoder<W>(
    builder: &mut EncodeBuilder,
    output: W,
    container: Container,
    channels: usize,
    sample_rate: u32,
    frame_count: u32,
) -> Result<Box<dyn Processor>>
where
    W: Write + Seek + 'static,
{
    match builder.codec {
        Codec::Atrac1 => build_atrac1_encoder(
            builder,
            build_atrac1_output(output, container, channels, frame_count)?,
        ),
        Codec::Atrac3 | Codec::Atrac3Lp4 => {
            let settings = build_atrac3_settings(builder, channels)?;
            let output = build_atrac3_output(
                output,
                container,
                channels,
                sample_rate,
                frame_count,
                settings,
            )?;
            build_atrac3_encoder(output, settings, builder.yaml_log.take())
        }
        Codec::Atrac3plus => unreachable!("ATRAC3plus is rejected before encoder construction"),
    }
}

fn build_atrac1_encoder(
    builder: &EncodeBuilder,
    output: Box<dyn CompressedOutput>,
) -> Result<Box<dyn Processor>> {
    let window_mode = match builder.at1.window_mode {
        At1WindowMode::Auto => WindowMode::Auto,
        At1WindowMode::NoTransient => WindowMode::NoTransient,
    };
    let settings = CoreAt1Settings::new(
        builder.at1.bfu_idx_const,
        window_mode,
        builder.at1.window_mask,
    )?;
    Ok(Box::new(Atrac1Encoder::try_new(output, settings)?))
}

fn build_atrac3_encoder(
    output: Box<dyn CompressedOutput>,
    settings: CoreAt3Settings,
    yaml_log: Option<Box<dyn Write>>,
) -> Result<Box<dyn Processor>> {
    Ok(Box::new(Atrac3Encoder::with_yaml_log(
        output, settings, yaml_log,
    )))
}

fn build_atrac3_settings(builder: &EncodeBuilder, channels: usize) -> Result<CoreAt3Settings> {
    let bitrate = match (builder.codec, builder.at3.bitrate_kbps) {
        (Codec::Atrac3Lp4, None) => LP4.bitrate,
        (_, bitrate) => bitrate.unwrap_or(0) * 1024,
    };
    let mut settings = CoreAt3Settings::new(
        bitrate,
        builder.at3.no_gain_control,
        builder.at3.no_tonal_components,
        channels as u8,
        builder.at3.bfu_idx_const,
    )
    .ok_or_else(|| invalid_input("unsupported ATRAC3 bitrate"))?;
    settings.bfu_alloc_mode = builder.at3.bfu_mode.into();
    Ok(settings)
}

fn build_atrac1_output<W>(
    output: W,
    container: Container,
    channels: usize,
    frame_count: u32,
) -> Result<Box<dyn CompressedOutput>>
where
    W: Write + Seek + 'static,
{
    match container {
        Container::Aea => Ok(Box::new(AeaOutput::new(
            output,
            "atracdenc-rust",
            channels,
            frame_count,
        )?)),
        Container::Raw => Ok(Box::new(RawOutput::new(
            output,
            channels,
            Some(AEA_FRAME_SIZE),
        ))),
        _ => unreachable!("ATRAC1 container validity checked earlier"),
    }
}

fn build_atrac3_output<W>(
    output: W,
    container: Container,
    channels: usize,
    sample_rate: u32,
    frame_count: u32,
    settings: CoreAt3Settings,
) -> Result<Box<dyn CompressedOutput>>
where
    W: Write + Seek + 'static,
{
    let params = settings.container_params;
    match container {
        Container::Oma => {
            let channel_format = if params.joint_stereo {
                OmaChannelFormat::StereoJoint
            } else {
                OmaChannelFormat::Stereo
            };
            Ok(Box::new(OmaOutput::new(
                output,
                OmaCodec::Atrac3,
                sample_rate,
                channel_format,
                params.frame_sz as u32,
            )?))
        }
        Container::Riff => Ok(Box::new(At3Output::atrac3(
            output,
            2,
            frame_count,
            params.frame_sz as u32,
            params.joint_stereo,
        )?)),
        Container::Rm => Ok(Box::new(RmOutput::new(
            output,
            channels,
            frame_count,
            params.frame_sz as u32,
            params.joint_stereo,
        )?)),
        Container::Raw => Ok(Box::new(RawOutput::new(
            output,
            channels,
            Some(params.frame_sz as usize),
        ))),
        _ => unreachable!("ATRAC3 container validity checked earlier"),
    }
}

fn validate_encode_settings(builder: &EncodeBuilder) -> Result<()> {
    if builder.codec != Codec::Atrac1 && builder.at1.window_mode == At1WindowMode::NoTransient {
        return Err(invalid_input("--notransient is only supported for atrac1"));
    }
    if builder.codec == Codec::Atrac1 && builder.yaml_log.is_some() {
        return Err(invalid_input("--yaml-log is only supported for ATRAC3"));
    }
    if builder.codec == Codec::Atrac1
        && (builder.at3.no_gain_control || builder.at3.no_tonal_components)
    {
        return Err(invalid_input(
            "--nogaincontrol and --notonal are only supported for atrac3",
        ));
    }
    if !matches!(builder.codec, Codec::Atrac3 | Codec::Atrac3Lp4)
        && builder.at3.bitrate_kbps.is_some()
    {
        return Err(invalid_input("--bitrate is only supported for atrac3"));
    }
    if let Some(bitrate) = builder.at3.bitrate_kbps
        && !(32..=384).contains(&bitrate)
    {
        return Err(invalid_input("--bitrate must be in the range 32..384"));
    }
    if builder.at3.bfu_idx_fast && builder.at3.bfu_mode == At3BfuMode::Parity {
        return Err(invalid_input(
            "--bfuidxfast cannot be combined with --at3-bfu-mode parity",
        ));
    }
    if builder.codec == Codec::Atrac1
        && builder.at1.bfu_idx_const > atracdenc_core::at1::data::MAX_BFU_IDX_CONST
    {
        // Surface the core invariant early (before opening the WAV) using the
        // canonical core error so the limit has a single source of truth.
        CoreAt1Settings::new(
            builder.at1.bfu_idx_const,
            WindowMode::Auto,
            builder.at1.window_mask,
        )?;
    }
    Ok(())
}

fn validate_wav<R: Read>(reader: &WavReader<R>) -> Result<()> {
    let channels = reader.channels();
    if channels == 0 || channels > 2 {
        return Err(invalid_input(
            "only mono and stereo WAV input is currently supported",
        ));
    }
    if reader.sample_rate() != 44_100 {
        return Err(invalid_input(
            "unsupported sample rate: only 44100 Hz is supported",
        ));
    }
    if reader.bits_per_sample() != 16 {
        return Err(invalid_input(
            "unsupported WAV format: only 16-bit PCM is supported",
        ));
    }
    Ok(())
}

pub fn validate_container(codec: Codec, container: Container) -> Result<()> {
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

fn default_container(codec: Codec) -> Container {
    match codec {
        Codec::Atrac1 => Container::Aea,
        Codec::Atrac3 | Codec::Atrac3Lp4 | Codec::Atrac3plus => Container::Oma,
    }
}

fn encoded_frame_count(total_samples: u64, codec: Codec) -> Result<u32> {
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

fn invalid_input(message: impl Into<String>) -> Error {
    Error::InvalidInput(message.into())
}

#[derive(Clone, Default)]
struct SharedCursor {
    inner: Rc<RefCell<Cursor<Vec<u8>>>>,
}

impl SharedCursor {
    fn into_bytes(self) -> Vec<u8> {
        self.inner.borrow().get_ref().clone()
    }
}

impl Write for SharedCursor {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.borrow_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.borrow_mut().flush()
    }
}

impl Seek for SharedCursor {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.borrow_mut().seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_bytes(frames: usize) -> Vec<u8> {
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
        bytes
    }

    #[test]
    fn validates_codec_container_matrix() {
        assert!(validate_container(Codec::Atrac1, Container::Aea).is_ok());
        assert!(validate_container(Codec::Atrac1, Container::Raw).is_ok());
        assert!(validate_container(Codec::Atrac3, Container::Rm).is_ok());
        assert_eq!(
            "invalid input: container oma is not supported for atrac1",
            validate_container(Codec::Atrac1, Container::Oma)
                .unwrap_err()
                .to_string()
        );
    }

    #[test]
    fn rejects_invalid_atrac1_bfu_idx_const() {
        let err = EncodeBuilder::new()
            .input_bytes(wav_bytes(1024))
            .container(Container::Aea)
            .at1_settings(At1Settings {
                bfu_idx_const: 9,
                ..At1Settings::default()
            })
            .run_to_vec()
            .unwrap_err();
        assert_eq!(
            "invalid input: bfu_idx_const must be in the range 0..=8 for atrac1",
            err.to_string()
        );
    }

    #[test]
    fn encodes_atrac1_from_bytes_to_vec() {
        let bytes = EncodeBuilder::new()
            .input_bytes(wav_bytes(8192))
            .codec(Codec::Atrac1)
            .container(Container::Aea)
            .run_to_vec()
            .unwrap();
        assert_eq!(&[0x00, 0x08, 0x00, 0x00], &bytes[..4]);
        assert!(bytes.len() > AEA_FRAME_SIZE);
    }

    #[test]
    fn encodes_atrac1_from_reader_to_vec() {
        let bytes = EncodeBuilder::new()
            .input_reader(Cursor::new(wav_bytes(8192)))
            .codec(Codec::Atrac1)
            .container(Container::Aea)
            .run_to_vec()
            .unwrap();
        assert_eq!(&[0x00, 0x08, 0x00, 0x00], &bytes[..4]);
        assert!(bytes.len() > AEA_FRAME_SIZE);
    }

    #[test]
    fn encodes_atrac1_to_writer_with_default_container() {
        let output = SharedCursor::default();
        EncodeBuilder::new()
            .input_bytes(wav_bytes(8192))
            .codec(Codec::Atrac1)
            .output_writer(output.clone())
            .run()
            .unwrap();

        let bytes = output.into_bytes();
        assert_eq!(&[0x00, 0x08, 0x00, 0x00], &bytes[..4]);
        assert!(bytes.len() > AEA_FRAME_SIZE);
    }

    #[test]
    fn encodes_atrac3_to_vec() {
        let bytes = EncodeBuilder::new()
            .input_bytes(wav_bytes(2048))
            .codec(Codec::Atrac3)
            .container(Container::Oma)
            .at3_settings(At3Settings {
                no_tonal_components: true,
                ..At3Settings::default()
            })
            .run_to_vec()
            .unwrap();
        assert_eq!(b"EA3", &bytes[..3]);
        assert!(bytes.len() > 96);
    }

    #[test]
    fn encodes_atrac3_to_vec_with_default_container() {
        let bytes = EncodeBuilder::new()
            .input_bytes(wav_bytes(2048))
            .codec(Codec::Atrac3)
            .at3_settings(At3Settings {
                no_tonal_components: true,
                ..At3Settings::default()
            })
            .run_to_vec()
            .unwrap();
        assert_eq!(b"EA3", &bytes[..3]);
        assert!(bytes.len() > 96);
    }

    #[test]
    fn decodes_atrac1_from_bytes_to_wav_vec() {
        let encoded = EncodeBuilder::new()
            .input_bytes(wav_bytes(8192))
            .codec(Codec::Atrac1)
            .container(Container::Aea)
            .run_to_vec()
            .unwrap();

        let decoded = DecodeBuilder::new()
            .codec(Codec::Atrac1)
            .input_bytes(encoded)
            .run_to_vec()
            .unwrap();

        assert_eq!(b"RIFF", &decoded[..4]);
        assert_eq!(b"WAVE", &decoded[8..12]);
    }

    #[test]
    fn run_requires_explicit_writer_output() {
        let err = EncodeBuilder::new()
            .input_bytes(wav_bytes(8192))
            .codec(Codec::Atrac1)
            .container(Container::Aea)
            .run()
            .unwrap_err();

        assert_eq!("invalid input: missing output file", err.to_string());
    }
}
