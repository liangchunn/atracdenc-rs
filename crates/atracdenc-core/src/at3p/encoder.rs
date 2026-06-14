//! ATRAC3+ encoder pipeline.
//!
//! Port of `atracdenc/src/atrac/at3p/at3p.cpp` + `atrac3p.h` (LGPL-2.1).
//! Per frame (2048 samples/channel): 16-band PQF analysis → GHA tonal
//! extraction (writing the residual) → per-subband MDCT of the residual →
//! scaling → bitstream, with a one-frame tonal-block delay.

use crate::atrac::scale::{BlockLayout, Scaler};
use crate::container::CompressedOutput;
use crate::error::AtracdencError;
use crate::pcm::engine::{ProcessMeta, ProcessResult, Processor};

use super::bitstream::{At3PBitStream, SingleChannelElement};
use super::gha::{At3PGhaData, GhaProcessor, make_gha_processor0};
use super::mdct::{At3pMdct, MdctHistBuf};
use super::pqf::At3pPqf;
use super::tables::{BLOCK_SIZE_TAB, BLOCKS_PER_BAND, SCALE_TABLE, SPECS_PER_BLOCK};

/// Samples per channel per frame.
pub const NUM_SAMPLES: usize = 2048;

/// GHA processing flags.
pub const GHA_PASS_INPUT: u8 = 1;
pub const GHA_WRITE_TONAL: u8 = 1 << 1;
pub const GHA_WRITE_RESIDUAL: u8 = 1 << 2;
pub const GHA_ENABLED: u8 = GHA_PASS_INPUT | GHA_WRITE_TONAL | GHA_WRITE_RESIDUAL;

/// Encoder settings (mirrors `TAt3PEnc::TSettings`).
#[derive(Debug, Clone, Copy)]
pub struct At3pSettings {
    pub use_gha: u8,
}

impl Default for At3pSettings {
    fn default() -> Self {
        Self {
            use_gha: GHA_ENABLED,
        }
    }
}

fn set_gha(s: &str, settings: &mut At3pSettings) -> Result<(), AtracdencError> {
    let mask: i32 = s
        .parse()
        .map_err(|_| AtracdencError::InvalidInput("invalid value of GHA processing mask".into()))?;
    if !(0..=7).contains(&mask) {
        return Err(AtracdencError::InvalidInput(
            "invalud value of GHA processing mask".into(),
        ));
    }
    settings.use_gha = mask as u8;
    Ok(())
}

/// Parse the `--advanced` option string (mirrors `ParseAdvancedOpt`). Only the
/// `ghadbg=<mask>` key is recognized.
pub fn parse_advanced_opt(
    opt: Option<&str>,
    settings: &mut At3pSettings,
) -> Result<(), AtracdencError> {
    let opt = match opt {
        Some(o) if !o.is_empty() => o,
        _ => return Ok(()),
    };

    for pair in opt.split(',') {
        if pair.is_empty() {
            return Err(AtracdencError::InvalidInput(
                "unexpected \",\" just after key.".into(),
            ));
        }
        let mut it = pair.splitn(2, '=');
        let key = it.next().unwrap();
        let val = match it.next() {
            Some(v) => v,
            None => {
                return Err(AtracdencError::InvalidInput(
                    "unexpected end of key token".into(),
                ));
            }
        };
        match key {
            "ghadbg" => set_gha(val, settings)?,
            other => {
                return Err(AtracdencError::InvalidInput(format!(
                    "unexpected advanced option \"{other}"
                )));
            }
        }
    }
    Ok(())
}

struct ChannelCtx {
    pqf: At3pPqf,
    buf: [[f32; NUM_SAMPLES]; 2],
    next_idx: usize,
    cur_idx: Option<usize>,
    prev_buf: [f32; NUM_SAMPLES],
    mdct_buf: MdctHistBuf,
    specs: Vec<f32>,
}

impl ChannelCtx {
    fn new() -> Self {
        Self {
            pqf: At3pPqf::new(),
            buf: [[0.0; NUM_SAMPLES]; 2],
            next_idx: 0,
            cur_idx: None,
            prev_buf: [0.0; NUM_SAMPLES],
            mdct_buf: [[0.0; 256]; 16],
            specs: vec![0.0; NUM_SAMPLES],
        }
    }
}

fn block_layout() -> (Vec<u8>, Vec<u16>, Vec<u16>, Vec<bool>) {
    let blocks_per_band: Vec<u8> = BLOCKS_PER_BAND.iter().map(|&v| v as u8).collect();
    let specs_start: Vec<u16> = BLOCK_SIZE_TAB.iter().map(|&v| v as u16).collect();
    let specs_per_block: Vec<u16> = SPECS_PER_BLOCK.iter().map(|&v| v as u16).collect();
    let short_windows = vec![false; 16];
    (blocks_per_band, specs_start, specs_per_block, short_windows)
}

/// ATRAC3+ encoder. Implements [`Processor`] driven with `NUM_SAMPLES` step.
pub struct At3pEncoder {
    output: Box<dyn CompressedOutput>,
    channels: usize,
    bitstream: At3PBitStream,
    scaler: Scaler,
    mdct: At3pMdct,
    gha: Box<dyn GhaProcessor>,
    ch: Vec<ChannelCtx>,
    delay: At3PGhaData,
    delay_present: bool,
    settings: At3pSettings,
    layout_blocks_per_band: Vec<u8>,
    layout_specs_start: Vec<u16>,
    layout_specs_per_block: Vec<u16>,
    layout_short_windows: Vec<bool>,
}

impl At3pEncoder {
    pub fn new(output: Box<dyn CompressedOutput>, channels: usize, settings: At3pSettings) -> Self {
        let (bpb, ss, spb, sw) = block_layout();
        Self {
            output,
            channels,
            bitstream: At3PBitStream::new(2048),
            scaler: Scaler::new(&*SCALE_TABLE),
            mdct: At3pMdct::new(),
            gha: make_gha_processor0(channels == 2),
            ch: (0..channels).map(|_| ChannelCtx::new()).collect(),
            delay: At3PGhaData::default(),
            delay_present: false,
            settings,
            layout_blocks_per_band: bpb,
            layout_specs_start: ss,
            layout_specs_per_block: spb,
            layout_short_windows: sw,
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    fn encode_frame(&mut self, data: &[f32]) -> Result<ProcessResult, AtracdencError> {
        let channels = self.channels;

        let mut need_more = 0;
        for ch in 0..channels {
            let mut src = [0.0f32; NUM_SAMPLES];
            for i in 0..NUM_SAMPLES {
                src[i] = data[i * channels + ch];
            }
            let c = &mut self.ch[ch];
            let next = c.next_idx;
            c.pqf.analyse(&src, &mut c.buf[next]);
            if c.cur_idx.is_none() {
                // CurBuf = Buf2; swap(NextBuf, CurBuf)
                let mut cur = 1usize;
                std::mem::swap(&mut c.next_idx, &mut cur);
                c.cur_idx = Some(cur);
                need_more += 1;
            }
        }

        if need_more == channels {
            return Ok(ProcessResult::LookAhead);
        }

        // GHA analysis: writes residual into each channel's prev_buf.
        // `buf` (read) and `prev_buf` (written) are disjoint fields, so the
        // band buffers are passed by reference without copying.
        let tonal_owned: Option<At3PGhaData> = if channels == 2 {
            let (l, r) = self.ch.split_at_mut(1);
            let c0 = &mut l[0];
            let c1 = &mut r[0];
            let cur0 = c0.cur_idx.unwrap();
            let next0 = c0.next_idx;
            let cur1 = c1.cur_idx.unwrap();
            let next1 = c1.next_idx;
            self.gha
                .do_analyze(
                    [&c0.buf[cur0], &c0.buf[next0]],
                    [&c1.buf[cur1], &c1.buf[next1]],
                    &mut c0.prev_buf,
                    &mut c1.prev_buf,
                )
                .cloned()
        } else {
            let mut dummy = [0.0f32; NUM_SAMPLES];
            let c0 = &mut self.ch[0];
            let cur0 = c0.cur_idx.unwrap();
            let next0 = c0.next_idx;
            self.gha
                .do_analyze(
                    [&c0.buf[cur0], &c0.buf[next0]],
                    [&[], &[]],
                    &mut c0.prev_buf,
                    &mut dummy,
                )
                .cloned()
        };

        // Old (delayed) tonal block for the bitstream.
        let write_residual = self.settings.use_gha & GHA_WRITE_RESIDUAL != 0;

        let mut sces = vec![SingleChannelElement::default(); channels];
        for ch in 0..channels {
            let win = sces[ch].subband_info.win;
            let mut tmp = [0.0f32; NUM_SAMPLES];
            if write_residual {
                let c = &self.ch[ch];
                for i in 0..NUM_SAMPLES {
                    tmp[i] = c.prev_buf[i] / (32768.0 / 1.122018);
                }
            }
            let bands: [&[f32]; 16] = std::array::from_fn(|b| &tmp[b * 128..b * 128 + 128]);
            let c = &mut self.ch[ch];
            self.mdct
                .do_mdct(&mut c.specs, &bands, &mut c.mdct_buf, win);
            let layout = BlockLayout {
                num_qmf: 16,
                blocks_per_band: &self.layout_blocks_per_band,
                specs_start_short: &self.layout_specs_start,
                specs_start_long: &self.layout_specs_start,
                specs_per_block: &self.layout_specs_per_block,
                short_windows: &self.layout_short_windows,
            };
            sces[ch].scaled_blocks = self.scaler.scale_frame(&c.specs, &layout);
        }

        let delay_ref = if self.delay_present {
            Some(&self.delay)
        } else {
            None
        };
        self.bitstream
            .write_frame(&mut *self.output, channels, delay_ref, &sces)?;

        for ch in 0..channels {
            let c = &mut self.ch[ch];
            if self.settings.use_gha & GHA_PASS_INPUT != 0 {
                let cur = c.cur_idx.unwrap();
                let copy = c.buf[cur];
                c.prev_buf.copy_from_slice(&copy);
            } else {
                c.prev_buf.fill(0.0);
            }
            let cur = c.cur_idx.unwrap();
            c.cur_idx = Some(c.next_idx);
            c.next_idx = cur;
        }

        match tonal_owned {
            Some(t) if self.settings.use_gha & GHA_WRITE_TONAL != 0 && t.num_tone_bands != 0 => {
                self.delay = t;
                self.delay_present = true;
            }
            _ => {
                self.delay_present = false;
            }
        }

        Ok(ProcessResult::Processed)
    }
}

impl Processor for At3pEncoder {
    fn process_frame(
        &mut self,
        data: &mut [f32],
        _meta: &ProcessMeta,
    ) -> Result<ProcessResult, AtracdencError> {
        self.encode_frame(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_advanced_ghadbg() {
        let mut s = At3pSettings::default();
        parse_advanced_opt(Some("ghadbg=5"), &mut s).unwrap();
        assert_eq!(s.use_gha, 5);
        assert!(parse_advanced_opt(Some("ghadbg=8"), &mut s).is_err());
        assert!(parse_advanced_opt(Some("unknown=1"), &mut s).is_err());
        let mut s2 = At3pSettings::default();
        parse_advanced_opt(None, &mut s2).unwrap();
        assert_eq!(s2.use_gha, GHA_ENABLED);
    }
}
