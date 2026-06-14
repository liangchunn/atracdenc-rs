//! ATRAC3+ bitstream writer.
//!
//! Port of `atracdenc/src/atrac/at3p/at3p_bitstream.{cpp,h}` and
//! `at3p_bitstream_impl.h` (LGPL-2.1). Builds an ATRAC3+ channel-unit frame:
//! word lengths, scale factors, quantized spectra (VLC), and the GHA tonal
//! block, with multi-pass bit budgeting via the `bitstream::encode` framework.

use std::sync::LazyLock;

use crate::atrac::scale::{ScaledBlock, quant_mantissas};
use crate::bitstream::BitStream;
use crate::bitstream::encode::{
    BitAllocHandler, BitStreamEncoder, BitStreamPartEncoder, EncodeStatus,
};
use crate::container::{CompressedOutput, ContainerError};

use super::ff_tables::*;
use super::gha::{At3PGhaData, WaveParam};
use super::mdct::At3pMdctWin;
use super::tables::{HuffTables, SPECS_PER_BLOCK, VlcElement};

static HUFF: LazyLock<HuffTables> = LazyLock::new(HuffTables::new);

// ----- frequency bit packing -----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TonePackOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy)]
pub struct TonePackEntry {
    pub code: u16,
    pub bits: u16,
}

#[derive(Debug, Clone)]
pub struct TonePackResult {
    pub data: Vec<TonePackEntry>,
    pub used_bits: i32,
    pub order: TonePackOrder,
}

fn get_first_set_bit(x: u32) -> u16 {
    if x == 0 {
        0
    } else {
        (31 - x.leading_zeros()) as u16
    }
}

pub fn create_freq_bit_pack(param: &[WaveParam]) -> TonePackResult {
    const MAX_BITS: i32 = 10;
    let len = param.len();
    let mut bits = [MAX_BITS, MAX_BITS];
    let mut res0: Vec<TonePackEntry> = Vec::with_capacity(len);
    let mut res1: Vec<TonePackEntry> = Vec::with_capacity(len);

    // ascending
    {
        let mut prev = (param[0].freq_index & 1023) as u16;
        res0.push(TonePackEntry {
            code: prev,
            bits: MAX_BITS as u16,
        });
        for i in 1..len {
            let cur = (param[i].freq_index & 1023) as u16;
            if prev < 512 {
                res0.push(TonePackEntry {
                    code: cur,
                    bits: MAX_BITS as u16,
                });
                bits[0] += MAX_BITS;
            } else {
                let b = get_first_set_bit(1023 - prev as u32) + 1;
                let code = cur as i32 - (1024 - (1 << b));
                res0.push(TonePackEntry {
                    code: code as u16,
                    bits: b,
                });
                bits[0] += b as i32;
            }
            prev = cur;
        }
    }

    // descending
    if len > 1 {
        let mut prev = (param[len - 1].freq_index & 1023) as u16;
        res1.push(TonePackEntry {
            code: prev,
            bits: MAX_BITS as u16,
        });
        for i in (0..=len - 2).rev() {
            let cur = (param[i].freq_index & 1023) as u16;
            let b = get_first_set_bit(prev as u32) + 1;
            res1.push(TonePackEntry { code: cur, bits: b });
            bits[1] += b as i32;
            prev = cur;
        }
    }

    if len == 1 || bits[0] < bits[1] {
        TonePackResult {
            data: res0,
            used_bits: bits[0],
            order: TonePackOrder::Asc,
        }
    } else {
        TonePackResult {
            data: res1,
            used_bits: bits[1],
            order: TonePackOrder::Desc,
        }
    }
}

// ----- frame model -----

#[derive(Clone, Default)]
pub struct SubbandInfos {
    pub win: At3pMdctWin,
}

#[derive(Clone, Default)]
pub struct SingleChannelElement {
    pub subband_info: SubbandInfos,
    pub scaled_blocks: Vec<ScaledBlock>,
}

pub struct SpecFrame<'a> {
    size_bits: u32,
    num_quant_units: u32,
    tonal_block: Option<&'a At3PGhaData>,
    word_len: Vec<(u8, u8)>,
    sf_idx: Vec<(u8, u8)>,
    spec_tab_idx: Vec<(u8, u8)>,
    channels: usize,
    sces: &'a [SingleChannelElement],
    #[allow(dead_code)]
    allocated_bits: usize,
}

impl<'a> SpecFrame<'a> {
    pub fn new(
        size_bits: u32,
        num_quant_units: u32,
        channels: usize,
        tonal_block: Option<&'a At3PGhaData>,
        sces: &'a [SingleChannelElement],
    ) -> Self {
        Self {
            size_bits,
            num_quant_units,
            tonal_block,
            word_len: Vec::new(),
            sf_idx: Vec::new(),
            spec_tab_idx: Vec::new(),
            channels,
            sces,
            allocated_bits: 0,
        }
    }
}

// ----- dumper helper -----

#[derive(Default)]
struct Dumper {
    buf: Vec<(u16, u8)>,
}

impl Dumper {
    fn insert(&mut self, value: u16, nbits: u8) {
        self.buf.push((value, nbits));
    }
    fn dump(&mut self, bs: &mut BitStream) {
        for &(v, n) in &self.buf {
            bs.write(v as u32, n as usize);
        }
        self.buf.clear();
    }
    fn reset(&mut self) {
        self.buf.clear();
    }
    fn consumption(&self) -> u32 {
        self.buf.iter().map(|&(_, n)| n as u32).sum()
    }
}

// ----- part: configure -----

#[derive(Default)]
struct Configure {
    d: Dumper,
}

const ALLOC_TABLE: [u8; 32] = [
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6, 6, 5, 5, 4, 3, 2, 1,
];

impl<'a> BitStreamPartEncoder<SpecFrame<'a>> for Configure {
    fn encode(&mut self, frame: &mut SpecFrame<'a>, _ba: &mut BitAllocHandler) -> EncodeStatus {
        let nqu = frame.num_quant_units as usize;
        frame.word_len.resize(nqu, (0, 0));
        for i in 0..nqu {
            frame.word_len[i] = (ALLOC_TABLE[i], ALLOC_TABLE[i]);
        }

        frame.sf_idx.resize(nqu, (0, 0));
        for i in 0..nqu {
            let first = frame.sces[0].scaled_blocks[i].scale_factor_index;
            let second = if frame.channels > 1 {
                frame.sces[1].scaled_blocks[i].scale_factor_index
            } else {
                0
            };
            frame.sf_idx[i] = (first, second);
        }

        frame.spec_tab_idx.resize(nqu, (0, 0));

        self.d.insert((frame.num_quant_units - 1) as u16, 5);
        self.d.insert(0, 1); // mute flag

        frame.allocated_bits = self.d.consumption() as usize;
        EncodeStatus::Ok
    }
    fn dump(&mut self, bs: &mut BitStream) {
        self.d.dump(bs);
    }
    fn reset(&mut self) {
        self.d.reset();
    }
    fn consumption(&self) -> u32 {
        self.d.consumption()
    }
}

// ----- part: word lengths -----

#[derive(Default)]
struct WordLenEncoder {
    d: Dumper,
}

fn find_best_wl_delta_encode(
    delta: &[u8],
    sz: usize,
    table_start: usize,
    table_end: usize,
) -> usize {
    let mut best = 0;
    let mut consumed = usize::MAX;
    for i in table_start..=table_end {
        let wl_tab = &HUFF.word_lens[i];
        let mut t = 0usize;
        for j in 1..sz {
            t += wl_tab[delta[j] as usize].len as usize;
        }
        if t < consumed {
            consumed = t;
            best = i;
        }
    }
    best
}

impl WordLenEncoder {
    fn vl_encode(&mut self, wl_tab: &[VlcElement; 8], idx: usize, sz: usize, data: &[u8]) {
        self.d.insert(3, 2); // VLC
        self.d.insert(0, 2); // weight_idx
        self.d.insert(0, 2); // num_coded_vals
        self.d.insert(idx as u16, 2);
        self.d.insert(data[0] as u16, 3);
        for i in 1..sz {
            let el = wl_tab[data[i] as usize];
            self.d.insert(el.code as u16, el.len as u8);
        }
    }
}

impl<'a> BitStreamPartEncoder<SpecFrame<'a>> for WordLenEncoder {
    fn encode(&mut self, frame: &mut SpecFrame<'a>, _ba: &mut BitAllocHandler) -> EncodeStatus {
        let nqu = frame.num_quant_units as usize;
        let mut deltas_ch0 = [0u8; 32];
        let mut inter_ch_deltas = [0u8; 32];
        let mut max_delta_ch0: i8 = 0;
        let mut max_inter_ch_delta: i8;

        {
            let t = frame.word_len[0].1 as i8 - frame.word_len[0].0 as i8;
            max_inter_ch_delta = t.abs();
            inter_ch_deltas[0] = (t & 7) as u8;
        }
        deltas_ch0[0] = frame.word_len[0].0;

        for i in 1..nqu {
            let delta_ch0 = frame.word_len[i].0 as i8 - frame.word_len[i - 1].0 as i8;
            let t = frame.word_len[i].1 as i8 - frame.word_len[i].0 as i8;
            max_delta_ch0 |= delta_ch0.abs();
            deltas_ch0[i] = (delta_ch0 & 7) as u8;
            max_inter_ch_delta |= t.abs();
            inter_ch_deltas[i] = (t & 7) as u8;
        }

        {
            let (ts, te) = if max_delta_ch0 >= 3 {
                (2, 3)
            } else if max_delta_ch0 == 2 {
                (1, 1)
            } else {
                (0, 0)
            };
            let idx = find_best_wl_delta_encode(&deltas_ch0, nqu, ts, te);
            let wl_tab = HUFF.word_lens[idx];
            self.vl_encode(&wl_tab, idx, nqu, &deltas_ch0);
        }

        if frame.channels == 2 {
            let (ts, te) = if max_inter_ch_delta >= 3 {
                (2, 3)
            } else if max_inter_ch_delta == 2 {
                (1, 1)
            } else {
                (0, 0)
            };
            let idx = find_best_wl_delta_encode(&inter_ch_deltas, nqu, ts, te);
            let wl_tab = HUFF.word_lens[idx];
            self.d.insert(1, 2);
            self.d.insert(0, 2);
            self.d.insert(idx as u16, 2);
            for i in 0..nqu {
                let el = wl_tab[inter_ch_deltas[i] as usize];
                self.d.insert(el.code as u16, el.len as u8);
            }
        }

        EncodeStatus::Ok
    }
    fn dump(&mut self, bs: &mut BitStream) {
        self.d.dump(bs);
    }
    fn reset(&mut self) {
        self.d.reset();
    }
    fn consumption(&self) -> u32 {
        self.d.consumption()
    }
}

// ----- part: scale factor indices -----

#[derive(Default)]
struct SfIdxEncoder {
    d: Dumper,
}

impl<'a> BitStreamPartEncoder<SpecFrame<'a>> for SfIdxEncoder {
    fn encode(&mut self, frame: &mut SpecFrame<'a>, _ba: &mut BitAllocHandler) -> EncodeStatus {
        if frame.sf_idx.is_empty() {
            return EncodeStatus::Ok;
        }
        let nqu = frame.num_quant_units as usize;
        for ch in 0..frame.channels {
            self.d.insert(0, 2); // constant number of bits
            for i in 0..nqu {
                let v = if ch == 0 {
                    frame.sf_idx[i].0
                } else {
                    frame.sf_idx[i].1
                };
                self.d.insert(v as u16, 6);
            }
        }
        EncodeStatus::Ok
    }
    fn dump(&mut self, bs: &mut BitStream) {
        self.d.dump(bs);
    }
    fn reset(&mut self) {
        self.d.reset();
    }
    fn consumption(&self) -> u32 {
        self.d.consumption()
    }
}

// ----- part: quant units (spectra) -----

#[derive(Default)]
struct QuantUnitsEncoder {
    d: Dumper,
}

fn encode_qu_spectra(qspec: &[i32], idx: usize, data: &mut Vec<(u16, u8)>) {
    let tab = &ATRAC3P_SPECTRA_TABS[idx];
    let vlc = &HUFF.vlc_specs[idx];
    let group_size = tab.group_size as usize;
    let num_coeffs = tab.num_coeffs as usize;
    let bits_coeff = tab.bits as usize;
    let is_signed = tab.is_signed != 0;
    let num_spec = qspec.len();

    let mut pos = 0usize;
    while pos < num_spec {
        if group_size != 1 {
            data.push((1, 1));
        }
        for _ in 0..group_size {
            let mut val: u32 = 0;
            let mut signs = [0i8; 4];
            for i in 0..num_coeffs {
                let mut t: i32 = qspec[pos];
                pos += 1;
                if !is_signed && t != 0 {
                    signs[i] = if t > 0 { 1 } else { -1 };
                    if t < 0 {
                        t = -t;
                    }
                } else {
                    t &= (1i32 << bits_coeff) - 1;
                }
                t <<= bits_coeff * i;
                val |= t as u32;
            }
            let el = vlc[val as usize];
            data.push((el.code as u16, el.len as u8));
            for i in 0..4 {
                if signs[i] != 0 {
                    data.push((if signs[i] > 0 { 0 } else { 1 }, 1));
                }
            }
        }
    }
}

fn encode_unit(values: &[f32], qu: usize, wordlen: usize, res: &mut Vec<(u16, u8)>) -> usize {
    let mul = 1.0 / ATRAC3P_MANT_TAB[wordlen];
    let mut mant = vec![0i32; SPECS_PER_BLOCK[qu] as usize];
    quant_mantissas(values, 0, mant.len() as u32, mul, false, &mut mant);

    let mut best_tab = 0usize;
    let mut consumed = usize::MAX;
    let mut tmp: Vec<(u16, u8)> = Vec::new();
    for i in 0..8 {
        let tab_index = wordlen - 1 + i * 7;
        tmp.clear();
        encode_qu_spectra(&mant, tab_index, &mut tmp);
        let t: usize = tmp.iter().map(|x| x.1 as usize).sum();
        if t < consumed {
            consumed = t;
            best_tab = i;
            res.clear();
            res.extend_from_slice(&tmp);
        }
    }
    best_tab
}

fn encode_code_tab(
    use_full_table: bool,
    channels: usize,
    num_quant_units: usize,
    spec_tab_idx: &[(u8, u8)],
    data: &mut Vec<(u16, u8)>,
) {
    data.push((use_full_table as u16, 1));
    let extra = use_full_table as u8;
    for ch in 0..channels {
        data.push((0, 1)); // table type
        data.push((0, 2)); // constant number of bits
        data.push((0, 1)); // num_coded_vals
        for i in 0..num_quant_units {
            let v = if ch == 0 {
                spec_tab_idx[i].0
            } else {
                spec_tab_idx[i].1
            };
            data.push((v as u16, extra + 2));
        }
    }
}

impl<'a> BitStreamPartEncoder<SpecFrame<'a>> for QuantUnitsEncoder {
    fn encode(&mut self, frame: &mut SpecFrame<'a>, _ba: &mut BitAllocHandler) -> EncodeStatus {
        let nqu = frame.num_quant_units as usize;
        let mut data: Vec<Vec<(u16, u8)>> = Vec::new();

        for ch in 0..frame.channels {
            let scaled_blocks = &frame.sces[ch].scaled_blocks;
            for qu in 0..nqu {
                let len = if ch == 0 {
                    frame.word_len[qu].0
                } else {
                    frame.word_len[qu].1
                } as usize;
                let values = &scaled_blocks[qu].values;
                let mut res = Vec::new();
                let tab_idx = encode_unit(values, qu, len, &mut res);
                if ch == 0 {
                    frame.spec_tab_idx[qu].0 = tab_idx as u8;
                } else {
                    frame.spec_tab_idx[qu].1 = tab_idx as u8;
                }
                data.push(res);
            }
            // power compensation placeholder group
            let num_pwr_spec =
                ATRAC3P_SUBBAND_TO_NUM_POWGRPS[ATRAC3P_QU_TO_SUBBAND[nqu - 1] as usize] as usize;
            let mut t = Vec::new();
            for _ in 0..num_pwr_spec {
                t.push((15u16, 4u8));
            }
            data.push(t);
        }

        {
            let mut tab_idx_data = Vec::new();
            encode_code_tab(
                true,
                frame.channels,
                nqu,
                &frame.spec_tab_idx,
                &mut tab_idx_data,
            );
            for (v, n) in tab_idx_data {
                self.d.insert(v, n);
            }
        }

        for x in &data {
            for &(v, n) in x {
                self.d.insert(v, n);
            }
        }

        EncodeStatus::Ok
    }
    fn dump(&mut self, bs: &mut BitStream) {
        self.d.dump(bs);
    }
    fn reset(&mut self) {
        self.d.reset();
    }
    fn consumption(&self) -> u32 {
        self.d.consumption()
    }
}

// ----- part: tonal components -----

#[derive(Default)]
struct TonalComponentEncoder {
    d: Dumper,
    bits_used: usize,
}

impl TonalComponentEncoder {
    fn write_subband_flags(&mut self, flags: &[bool]) {
        let sum: usize = flags.iter().map(|&f| f as usize).sum();
        if sum == 0 {
            self.d.insert(0, 1);
        } else if sum == flags.len() {
            self.d.insert(1, 1);
            self.d.insert(0, 1);
        } else {
            self.d.insert(1, 1);
            self.d.insert(1, 1);
            for &f in flags {
                self.d.insert(f as u16, 1);
            }
        }
    }

    fn write_tonal_block(&mut self, channels: usize, tonal: &At3PGhaData) {
        self.d.insert(1, 1); // amplitude mode 1

        let num_bands = tonal.num_tone_bands as usize;
        let tb_huff = HUFF.num_tone_bands[num_bands - 1];
        self.d.insert(tb_huff.code as u16, tb_huff.len as u8);

        if channels == 2 {
            self.write_subband_flags(&tonal.tone_sharing[..num_bands]);
            self.write_subband_flags(&[tonal.second_is_leader]);
            self.d.insert(0, 1);
        }

        for ch in 0..channels {
            if ch != 0 {
                self.d.insert(0, 1); // each channel has own envelope
            }
            // envelope data
            for i in 0..num_bands {
                if ch != 0 && tonal.tone_sharing[i] {
                    continue;
                }
                let env = tonal.envelope(ch, i);
                if env.0 != At3PGhaData::EMPTY_POINT {
                    self.d.insert(1, 1);
                    self.d.insert(env.0 as u16, 5);
                } else {
                    self.d.insert(0, 1);
                }
                if env.1 != At3PGhaData::EMPTY_POINT {
                    self.d.insert(1, 1);
                    self.d.insert(env.1 as u16, 5);
                } else {
                    self.d.insert(0, 1);
                }
            }

            // num waves
            self.d.insert(0, (ch + 1) as u8); // mode
            for i in 0..num_bands {
                if ch != 0 && tonal.tone_sharing[i] {
                    continue;
                }
                self.d.insert(tonal.num_waves(ch, i) as u16, 4);
            }

            // tone freq
            if ch != 0 {
                self.d.insert(0, 1);
            }
            for i in 0..num_bands {
                if ch != 0 && tonal.tone_sharing[i] {
                    continue;
                }
                let num_waves = tonal.num_waves(ch, i);
                if num_waves == 0 {
                    continue;
                }
                let (w, n) = tonal.waves(ch, i);
                let pkt = create_freq_bit_pack(&w[..n]);
                if num_waves > 1 {
                    self.d.insert((pkt.order == TonePackOrder::Desc) as u16, 1);
                }
                for e in &pkt.data {
                    self.d.insert(e.code, e.bits as u8);
                }
            }

            // amplitude
            self.d.insert(0, (ch + 1) as u8); // mode
            for i in 0..num_bands {
                if ch != 0 && tonal.tone_sharing[i] {
                    continue;
                }
                let num_waves = tonal.num_waves(ch, i);
                if num_waves == 0 {
                    continue;
                }
                let (w, n) = tonal.waves(ch, i);
                for j in 0..n {
                    self.d.insert(w[j].amp_sf as u16, 6);
                }
            }

            // phase
            for i in 0..num_bands {
                if ch != 0 && tonal.tone_sharing[i] {
                    continue;
                }
                let num_waves = tonal.num_waves(ch, i);
                if num_waves == 0 {
                    continue;
                }
                let (w, n) = tonal.waves(ch, i);
                for j in 0..n {
                    self.d.insert(w[j].phase_index as u16, 5);
                }
            }
        }
    }

    fn check_frame_done(&self, frame: &mut SpecFrame, ba: &BitAllocHandler) -> EncodeStatus {
        let total = self.bits_used as u32 + ba.cur_global_consumption();
        if total > frame.size_bits {
            if frame.num_quant_units == 32 {
                frame.num_quant_units = 28;
            } else {
                frame.num_quant_units -= 1;
            }
            EncodeStatus::Repeat
        } else {
            EncodeStatus::Ok
        }
    }
}

impl<'a> BitStreamPartEncoder<SpecFrame<'a>> for TonalComponentEncoder {
    fn encode(&mut self, frame: &mut SpecFrame<'a>, ba: &mut BitAllocHandler) -> EncodeStatus {
        if self.bits_used != 0 {
            if let Some(tb) = frame.tonal_block {
                if tb.num_tone_bands as u32 > frame.num_quant_units {
                    panic!("tonal bands exceed quant units (TODO)");
                }
            }
            return self.check_frame_done(frame, ba);
        }

        let ch_num = frame.channels;
        if ch_num == 2 {
            self.d.insert(0, 2); // swap_channels and negate_coeffs
        }

        for ch in 0..ch_num {
            let win = frame.sces[ch].subband_info.win;
            let sb_num = ATRAC3P_QU_TO_SUBBAND[frame.num_quant_units as usize - 1] + 1;
            if win.is_all_sine() {
                self.d.insert(0, 1);
            } else if win.is_all_steep(sb_num) {
                self.d.insert(1, 1);
                self.d.insert(0, 1);
            } else {
                self.d.insert(1, 1);
                self.d.insert(1, 1);
                for i in 0..sb_num {
                    self.d.insert(win.is_sb_steep(i) as u16, 1);
                }
            }
        }

        for _ in 0..ch_num {
            self.d.insert(0, 1); // gain comp
        }

        match frame.tonal_block {
            Some(tb) if tb.num_tone_bands != 0 => {
                self.d.insert(1, 1);
                self.write_tonal_block(ch_num, tb);
            }
            _ => {
                self.d.insert(0, 1);
            }
        }

        self.d.insert(0, 1); // no noise info
        self.d.insert(3, 2); // terminator

        self.bits_used = self.d.consumption() as usize;

        self.check_frame_done(frame, ba)
    }
    fn dump(&mut self, bs: &mut BitStream) {
        self.d.dump(bs);
        self.bits_used = 0;
    }
    fn reset(&mut self) {
        self.d.reset();
    }
    fn consumption(&self) -> u32 {
        self.d.consumption()
    }
}

fn create_enc_parts<'a>() -> Vec<Box<dyn BitStreamPartEncoder<SpecFrame<'a>>>> {
    vec![
        Box::new(Configure::default()),
        Box::new(WordLenEncoder::default()),
        Box::new(SfIdxEncoder::default()),
        Box::new(QuantUnitsEncoder::default()),
        Box::new(TonalComponentEncoder::default()),
    ]
}

/// ATRAC3+ frame bitstream writer.
pub struct At3PBitStream {
    frame_sz_to_alloc_bits: u32,
    frame_sz: u16,
}

impl At3PBitStream {
    pub fn new(frame_sz: u16) -> Self {
        Self {
            frame_sz_to_alloc_bits: frame_sz as u32 * 8 - 3,
            frame_sz,
        }
    }

    pub fn write_frame(
        &mut self,
        out: &mut dyn CompressedOutput,
        channels: usize,
        tonal: Option<&At3PGhaData>,
        sces: &[SingleChannelElement],
    ) -> Result<(), ContainerError> {
        let mut bs = BitStream::new();
        bs.write(0, 1);
        bs.write((channels - 1) as u32, 2);

        let mut frame = SpecFrame::new(self.frame_sz_to_alloc_bits, 32, channels, tonal, sces);
        let mut encoder = BitStreamEncoder::new(create_enc_parts());
        encoder.run(&mut frame, &mut bs);

        assert!(bs.size_in_bits() <= self.frame_sz as usize * 8);
        let mut buf = bs.bytes().to_vec();
        buf.resize(self.frame_sz as usize, 0);
        out.write_frame(&buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wp(freq: u32) -> WaveParam {
        WaveParam {
            freq_index: freq,
            amp_sf: 0,
            amp_index: 0,
            phase_index: 0,
        }
    }

    #[test]
    fn tone_freq_bit_pack_1() {
        let r = create_freq_bit_pack(&[wp(1)]);
        assert_eq!(r.used_bits, 10);
        assert_eq!(r.order, TonePackOrder::Asc);
        assert_eq!(r.data.len(), 1);
        assert_eq!(r.data[0].code, 1);
        assert_eq!(r.data[0].bits, 10);
    }

    #[test]
    fn tone_freq_bit_pack_512_1020_1023() {
        let r = create_freq_bit_pack(&[wp(512), wp(1020), wp(1023)]);
        assert_eq!(r.used_bits, 21);
        assert_eq!(r.order, TonePackOrder::Asc);
        assert_eq!(r.data.len(), 3);
        assert_eq!((r.data[0].code, r.data[0].bits), (512, 10));
        assert_eq!((r.data[1].code, r.data[1].bits), (508, 9));
        assert_eq!((r.data[2].code, r.data[2].bits), (3, 2));
    }

    #[test]
    fn tone_freq_bit_pack_1_2_3() {
        let r = create_freq_bit_pack(&[wp(1), wp(2), wp(3)]);
        assert_eq!(r.used_bits, 14);
        assert_eq!(r.order, TonePackOrder::Desc);
        assert_eq!(r.data.len(), 3);
        assert_eq!((r.data[0].code, r.data[0].bits), (3, 10));
        assert_eq!((r.data[1].code, r.data[1].bits), (2, 2));
        assert_eq!((r.data[2].code, r.data[2].bits), (1, 2));
    }

    #[test]
    fn tone_freq_bit_pack_1_2_3_1020_1021_1022() {
        let r = create_freq_bit_pack(&[wp(1), wp(2), wp(3), wp(1020), wp(1021), wp(1022)]);
        assert_eq!(r.used_bits, 44);
        assert_eq!(r.order, TonePackOrder::Desc);
        assert_eq!(r.data.len(), 6);
        assert_eq!((r.data[0].code, r.data[0].bits), (1022, 10));
        assert_eq!((r.data[1].code, r.data[1].bits), (1021, 10));
        assert_eq!((r.data[2].code, r.data[2].bits), (1020, 10));
        assert_eq!((r.data[3].code, r.data[3].bits), (3, 10));
        assert_eq!((r.data[4].code, r.data[4].bits), (2, 2));
        assert_eq!((r.data[5].code, r.data[5].bits), (1, 2));
    }

    #[test]
    fn tone_freq_bit_pack_1_2_1020_1021_1022() {
        let r = create_freq_bit_pack(&[wp(1), wp(2), wp(1020), wp(1021), wp(1022)]);
        assert_eq!(r.used_bits, 34);
        assert_eq!(r.order, TonePackOrder::Asc);
        assert_eq!(r.data.len(), 5);
        assert_eq!((r.data[0].code, r.data[0].bits), (1, 10));
        assert_eq!((r.data[1].code, r.data[1].bits), (2, 10));
        assert_eq!((r.data[2].code, r.data[2].bits), (1020, 10));
        assert_eq!((r.data[3].code, r.data[3].bits), (1, 2));
        assert_eq!((r.data[4].code, r.data[4].bits), (2, 2));
    }

    #[test]
    fn wordlen() {
        let sce = vec![
            SingleChannelElement::default(),
            SingleChannelElement::default(),
        ];
        let mut frame = SpecFrame::new(444, 28, 2, None, &sce);
        frame.num_quant_units = 6;
        for _ in 0..6 {
            frame.word_len.push((6, 6));
        }

        let mut bs = BitStream::new();
        let mut encoder: BitStreamEncoder<SpecFrame> =
            BitStreamEncoder::new(vec![Box::new(WordLenEncoder::default())]);
        encoder.run(&mut frame, &mut bs);

        assert_eq!(bs.size_in_bits(), 28);
    }
}
