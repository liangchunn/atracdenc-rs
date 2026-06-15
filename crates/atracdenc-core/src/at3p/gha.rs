//! ATRAC3+ GHA tonal analysis.
//!
//! Port of `atracdenc/src/atrac/at3p/at3p_gha.{h,cpp}` (LGPL-2.1). Extracts
//! sinusoidal (tonal) components from the lowest 8 PQF subbands using the
//! `gha` library, tracks tones across frames, decides stereo leader/sharing,
//! and subtracts the synthesized tones (via `ff_dsp`) to produce the residual.

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::sync::LazyLock;

use super::ff_dsp::{self, ChanUnitCtx};
use crate::atrac::psy::calc_ath;
use crate::gha::{GhaCtx, GhaInfo};

const SUBBANDS: usize = 8;
const SAMPLES_PER_SUBBAND: usize = 128;
const LOOK_AHEAD: usize = 64;
const GHA_SUBBAND_BUF_SZ: usize = SAMPLES_PER_SUBBAND + LOOK_AHEAD; // 192
const CHANNEL_BUF_SZ: usize = SUBBANDS * GHA_SUBBAND_BUF_SZ;

// ----- output data structure -----

/// A single extracted sine wave.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WaveParam {
    pub freq_index: u32,
    pub amp_sf: u32,
    pub amp_index: u32,
    pub phase_index: u32,
}

/// Per-subband wave grouping info.
#[derive(Debug, Clone, Copy)]
pub struct WaveSbInfo {
    pub wave_index: usize,
    pub wave_nums: usize,
    pub envelope: (u32, u32),
}

impl Default for WaveSbInfo {
    fn default() -> Self {
        Self {
            wave_index: 0,
            wave_nums: 0,
            envelope: (At3PGhaData::EMPTY_POINT, At3PGhaData::EMPTY_POINT),
        }
    }
}

/// Per-channel waves.
#[derive(Debug, Clone, Default)]
pub struct WavesChannel {
    pub wave_sb_infos: Vec<WaveSbInfo>,
    pub wave_params: Vec<WaveParam>,
}

/// GHA analysis result for one frame (mirrors `TAt3PGhaData`).
#[derive(Debug, Clone)]
pub struct At3PGhaData {
    pub waves: [WavesChannel; 2],
    pub num_tone_bands: u8,
    pub tone_sharing: [bool; 16],
    pub second_is_leader: bool,
}

impl Default for At3PGhaData {
    fn default() -> Self {
        Self {
            waves: [WavesChannel::default(), WavesChannel::default()],
            num_tone_bands: 0,
            tone_sharing: [false; 16],
            second_is_leader: false,
        }
    }
}

impl At3PGhaData {
    pub const EMPTY_POINT: u32 = u32::MAX;
    pub const INIT: u32 = u32::MAX - 1;

    pub fn num_waves(&self, ch: usize, sb: usize) -> usize {
        self.waves[ch].wave_sb_infos[sb].wave_nums
    }

    pub fn envelope(&self, ch: usize, sb: usize) -> (u32, u32) {
        self.waves[ch].wave_sb_infos[sb].envelope
    }

    pub fn waves(&self, ch: usize, sb: usize) -> (&[WaveParam], usize) {
        let info = &self.waves[ch].wave_sb_infos[sb];
        (
            &self.waves[ch].wave_params[info.wave_index..],
            info.wave_nums,
        )
    }
}

/// Processor interface (mirrors `IGhaProcessor`).
pub trait GhaProcessor {
    /// `b1`/`b2`: current & next subband buffers per channel; `w1`/`w2`:
    /// residual outputs. Returns the tonal data (borrowed) or `None`.
    fn do_analyze(
        &mut self,
        b1: [&[f32]; 2],
        b2: [&[f32]; 2],
        w1: &mut [f32],
        w2: &mut [f32],
    ) -> Option<&At3PGhaData>;
}

pub fn make_gha_processor0(stereo: bool) -> Box<dyn GhaProcessor> {
    Box::new(TGhaProcessor::new(stereo))
}

// ----- static tables -----

static SINE_TAB: LazyLock<[f32; 2048]> = LazyLock::new(|| {
    let mut t = [0.0f32; 2048];
    for i in 0..2048 {
        t[i] = (2.0 * PI * i as f64 / 2048.0).sin() as f32;
    }
    t
});

static AMP_SF_TAB: LazyLock<[f32; 64]> = LazyLock::new(|| {
    let mut t = [0.0f32; 64];
    for i in 0..64 {
        t[i] = ((i as f32 - 3.0) / 4.0).exp2();
    }
    t
});

static SUBBAND_ATH: LazyLock<[f32; SUBBANDS]> = LazyLock::new(|| {
    let ath = calc_ath(16 * 1024, 44100);
    let mut out = [0.0f32; SUBBANDS];
    for sb in 0..SUBBANDS {
        let mut m = 999.0f32;
        for i in 0..1024 {
            m = m.min(ath[sb * 1024 + i]);
        }
        out[sb] = 10.0f32.powf(0.1 * (m + 90.0));
    }
    out
});

// ----- index helpers -----

fn gha_freq_to_index(f: f32, sb: u32) -> u32 {
    let v = (1024.0_f64 * (f as f64 / PI)) as f32;
    ((v.round_ties_even() as i64 & 1023) as u32) | (sb << 10)
}

fn gha_phase_to_index(p: f32) -> u32 {
    let v = (32.0_f64 * (p as f64 / (2.0 * PI))) as f32;
    (v.round_ties_even() as i64 & 31) as u32
}

fn amplitude_to_sf(amp: f32) -> u32 {
    // upper_bound: first index whose value > amp, then step back one.
    let tab = &*AMP_SF_TAB;
    let mut idx = tab.partition_point(|&x| x <= amp);
    if idx != 0 {
        idx -= 1;
    }
    idx as u32
}

// ----- per-channel working state -----

struct TChannelData {
    buf: [f32; CHANNEL_BUF_SZ],
    envelopes: [(u32, u32); SUBBANDS],
    gapless: [bool; SUBBANDS],
    subband_done: [u8; SUBBANDS],
    gha_infos: BTreeMap<u32, GhaInfo>,
    max_tone_magnitude: [f32; SUBBANDS],
    last_residual_energy: [f32; SUBBANDS],
    last_added_freq_idx: [u32; SUBBANDS],
}

impl TChannelData {
    fn new() -> Self {
        Self {
            buf: [0.0; CHANNEL_BUF_SZ],
            envelopes: [(At3PGhaData::INIT, At3PGhaData::INIT); SUBBANDS],
            gapless: [false; SUBBANDS],
            subband_done: [0; SUBBANDS],
            gha_infos: BTreeMap::new(),
            max_tone_magnitude: [0.0; SUBBANDS],
            last_residual_energy: [0.0; SUBBANDS],
            last_added_freq_idx: [0; SUBBANDS],
        }
    }

    fn mark_subband_done(&mut self, sb: usize) {
        self.subband_done[sb] = 16;
    }

    fn is_subband_done(&self, sb: usize) -> bool {
        self.subband_done[sb] == 16
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdjustStatus {
    Error,
    Ok,
    Repeat,
}

struct CbState {
    adjust_status: AdjustStatus,
    frame_sz: usize,
}

// ----- the processor -----

struct TGhaProcessor {
    gha: GhaCtx,
    stereo: bool,
    result_buf: At3PGhaData,
    result_buf_history: At3PGhaData,
    ch_unit: ChanUnitCtx,
}

impl TGhaProcessor {
    fn new(stereo: bool) -> Self {
        let mut gha = GhaCtx::new(128);
        gha.set_max_magnitude(32768.0);
        gha.set_upsample(true);
        // force table init
        LazyLock::force(&SINE_TAB);
        LazyLock::force(&AMP_SF_TAB);
        LazyLock::force(&SUBBAND_ATH);

        Self {
            gha,
            stereo,
            result_buf: At3PGhaData::default(),
            result_buf_history: At3PGhaData::default(),
            ch_unit: ChanUnitCtx::default(),
        }
    }
}

/// FFmpeg `CheckResuidalAndApply`: analyze residual quality, set envelope and
/// adjust status, and store the residual back into the per-band buffer.
fn check_residual_and_apply(
    data: &mut TChannelData,
    sb: usize,
    src_buf: &[f32],
    cb: &mut CbState,
    residual: &[f32],
) {
    let src = &src_buf[sb * SAMPLES_PER_SUBBAND..];
    let mut residual_energy = 0.0f32;

    let mut start = 0u32;
    let mut cur_start = 0u32;
    let mut count = 0u32;
    let mut len = 0u32;
    let mut found = false;

    assert_eq!(residual.len_min(SAMPLES_PER_SUBBAND), SAMPLES_PER_SUBBAND);

    let mut i = 0usize;
    while i < SAMPLES_PER_SUBBAND {
        let mut energy_in = 0.0f32;
        let mut energy_out = 0.0f32;
        for j in 0..4 {
            energy_in += src[i + j] * src[i + j];
            energy_out += residual[i + j] * residual[i + j];
        }
        energy_in = (energy_in / 4.0).sqrt();
        energy_out = (energy_out / 4.0).sqrt();
        residual_energy += energy_out;

        if energy_in / energy_out < 1.0 {
            count = 0;
            found = false;
            cur_start = i as u32 + 4;
        } else {
            count += 1;
            if count > len {
                len = count;
                if !found {
                    start = cur_start;
                    found = true;
                }
            }
        }
        i += 4;
    }

    if len < 4 {
        cb.adjust_status = AdjustStatus::Error;
        return;
    }

    let end = start + len * 4;

    if cb.adjust_status != AdjustStatus::Repeat && end != SAMPLES_PER_SUBBAND as u32 {
        cb.frame_sz = end as usize;
        cb.adjust_status = AdjustStatus::Repeat;
        return;
    }

    let threshold = 1.05f32;
    if data.last_residual_energy[sb] == 0.0 {
        data.last_residual_energy[sb] = residual_energy;
    } else if data.last_residual_energy[sb] < residual_energy * threshold {
        cb.adjust_status = AdjustStatus::Error;
        return;
    } else {
        data.last_residual_energy[sb] = residual_energy;
    }

    data.envelopes[sb].0 = start;

    if data.envelopes[sb].1 == At3PGhaData::EMPTY_POINT && end != SAMPLES_PER_SUBBAND as u32 {
        cb.adjust_status = AdjustStatus::Error;
        return;
    }

    data.envelopes[sb].1 = end;
    cb.adjust_status = AdjustStatus::Ok;

    let b = &mut data.buf[sb * GHA_SUBBAND_BUF_SZ..sb * GHA_SUBBAND_BUF_SZ + SAMPLES_PER_SUBBAND];
    b.copy_from_slice(&residual[..SAMPLES_PER_SUBBAND]);
}

trait LenMin {
    fn len_min(&self, m: usize) -> usize;
}
impl LenMin for [f32] {
    fn len_min(&self, m: usize) -> usize {
        self.len().min(m)
    }
}

fn gen_waves(params: &[WaveParam], reg_offset: i32, out: &mut [f32], out_limit: usize) {
    let sine = &*SINE_TAB;
    let amp_tab = &*AMP_SF_TAB;
    for p in params {
        let amp = amp_tab[p.amp_sf as usize];
        let inc = p.freq_index as i32;
        let mut pos =
            (ff_dsp::dequant_phase(p.phase_index as i32) + (reg_offset ^ 128) * inc) & 2047;
        for i in 0..out_limit {
            out[i] += sine[pos as usize] * amp;
            pos = (pos + inc) & 2047;
        }
    }
}

fn check_next_frame(next_src: &[f32], gha_infos: &[GhaInfo]) -> bool {
    let mut t: Vec<WaveParam> = Vec::with_capacity(gha_infos.len());
    for x in gha_infos {
        t.push(WaveParam {
            freq_index: gha_freq_to_index(x.frequency, 0),
            amp_sf: amplitude_to_sf(x.magnitude),
            amp_index: 1,
            phase_index: gha_phase_to_index(x.phase),
        });
    }

    let mut buf = [0.0f32; LOOK_AHEAD];
    gen_waves(&t, 0, &mut buf, LOOK_AHEAD);

    let mut energy_before = 0.0f32;
    let mut energy_after = 0.0f32;
    for i in 0..LOOK_AHEAD {
        energy_before += next_src[i] * next_src[i];
        let d = next_src[i] - buf[i];
        energy_after += d * d;
    }
    energy_after < energy_before
}

fn psy_pre_check(sb: usize, gha: &GhaInfo, data: &TChannelData) -> bool {
    if gha.magnitude.is_nan() {
        return false;
    }
    if (gha.magnitude * gha.magnitude) > SUBBAND_ATH[sb] {
        if gha.magnitude > data.max_tone_magnitude[sb] / 10.0 {
            return true;
        }
    }
    false
}

fn adjust_envelope(src: (u32, u32), history: u32) -> (u32, u32) {
    let first = if src.0 == 0 && history == At3PGhaData::EMPTY_POINT {
        At3PGhaData::EMPTY_POINT
    } else {
        assert_ne!(src.0, At3PGhaData::EMPTY_POINT, "impossible envelope start");
        src.0 / 4
    };
    let second = if src.1 == At3PGhaData::EMPTY_POINT {
        src.1
    } else {
        assert_ne!(src.1, 0, "impossible envelope stop");
        let v = (src.1 - 1) / 4;
        assert!(v < 32, "envelope stop out of range");
        v
    };
    (first, second)
}

impl TGhaProcessor {
    fn do_round(
        &mut self,
        data: &mut TChannelData,
        src_buf: &[f32],
        src_buf_next: &[f32],
        total_tones: &mut usize,
    ) -> bool {
        let mut progress = false;
        for sb in 0..SUBBANDS {
            if data.is_subband_done(sb) {
                continue;
            }
            if *total_tones >= 48 {
                return false;
            }

            let src_b = &src_buf[sb * SAMPLES_PER_SUBBAND..];

            // ---- adjust existing tones in this subband ----
            let lo = (sb as u32) << 10;
            let hi = ((sb as u32) + 1) << 10;
            let mut tmp: Vec<GhaInfo> = data.gha_infos.range(lo..hi).map(|(_, v)| *v).collect();

            if !tmp.is_empty() {
                let mut cb = CbState {
                    adjust_status: AdjustStatus::Ok,
                    frame_sz: 0,
                };
                loop {
                    let n = tmp.len();
                    let frame_sz = cb.frame_sz;
                    let mut had_error = false;
                    {
                        let data_ref = &mut *data;
                        let cb_ref = &mut cb;
                        let closure = |residual: &[f32]| {
                            check_residual_and_apply(data_ref, sb, src_buf, cb_ref, residual);
                        };
                        let ar = self
                            .gha
                            .adjust_info(src_b, &mut tmp, n, frame_sz, Some(closure));
                        if ar < 0 {
                            had_error = true;
                        }
                    }
                    if had_error {
                        cb.adjust_status = AdjustStatus::Error;
                    }
                    if cb.adjust_status != AdjustStatus::Repeat {
                        break;
                    }
                }

                if cb.adjust_status == AdjustStatus::Ok {
                    tmp.sort_by(|a, b| a.frequency.partial_cmp(&b.frequency).unwrap());

                    let mut dup_found = false;
                    {
                        let mut idx1 = gha_freq_to_index(tmp[0].frequency, sb as u32);
                        for i in 1..tmp.len() {
                            let idx2 = gha_freq_to_index(tmp[i].frequency, sb as u32);
                            if idx2 == idx1 {
                                dup_found = true;
                                break;
                            } else {
                                idx1 = idx2;
                            }
                        }
                    }

                    if !dup_found {
                        if data.envelopes[sb].1 == SAMPLES_PER_SUBBAND as u32
                            || data.envelopes[sb].1 == At3PGhaData::EMPTY_POINT
                        {
                            let cont =
                                check_next_frame(&src_buf_next[SAMPLES_PER_SUBBAND * sb..], &tmp);
                            if data.gapless[sb] && !cont {
                                data.gha_infos.remove(&data.last_added_freq_idx[sb]);
                                *total_tones -= 1;
                                data.mark_subband_done(sb);
                                continue;
                            } else if data.envelopes[sb].1 == SAMPLES_PER_SUBBAND as u32 && cont {
                                data.envelopes[sb].1 = At3PGhaData::EMPTY_POINT;
                                data.gapless[sb] = true;
                            }
                        }

                        // erase old range entries, insert adjusted
                        let keys: Vec<u32> =
                            data.gha_infos.range(lo..hi).map(|(k, _)| *k).collect();
                        for k in keys {
                            data.gha_infos.remove(&k);
                        }
                        for x in &tmp {
                            data.max_tone_magnitude[sb] =
                                data.max_tone_magnitude[sb].max(x.magnitude);
                            let new_index = gha_freq_to_index(x.frequency, sb as u32);
                            data.gha_infos.insert(new_index, *x);
                        }
                    } else {
                        data.gha_infos.remove(&data.last_added_freq_idx[sb]);
                        *total_tones -= 1;
                        data.mark_subband_done(sb);
                        continue;
                    }
                } else {
                    data.gha_infos.remove(&data.last_added_freq_idx[sb]);
                    *total_tones -= 1;
                    data.mark_subband_done(sb);
                    continue;
                }
            }

            // ---- analyze a new tone ----
            let b = &data.buf[sb * GHA_SUBBAND_BUF_SZ..];
            let mut res = GhaInfo::default();
            self.gha.analyze_one(b, &mut res);

            let freq_index = gha_freq_to_index(res.frequency, sb as u32);
            if !psy_pre_check(sb, &res, data) {
                data.mark_subband_done(sb);
            } else {
                if data.subband_done[sb] == 0 {
                    let prev = data.gha_infos.insert(freq_index, res);
                    data.last_added_freq_idx[sb] = freq_index;
                    assert!(prev.is_none());
                } else {
                    const MIN_FREQ_DISTANCE: u32 = 20;
                    if let Some((k, _)) = data.gha_infos.range(freq_index..).next() {
                        if *k == freq_index {
                            data.mark_subband_done(sb);
                            continue;
                        }
                        if *k - freq_index < MIN_FREQ_DISTANCE {
                            data.mark_subband_done(sb);
                            continue;
                        }
                    }
                    if let Some((k, _)) = data.gha_infos.range(..freq_index).next_back() {
                        if freq_index - *k < MIN_FREQ_DISTANCE {
                            data.mark_subband_done(sb);
                            continue;
                        }
                    }
                    if data.subband_done[sb] == 15 {
                        data.mark_subband_done(sb);
                        continue;
                    }
                    data.gha_infos.insert(freq_index, res);
                    data.last_added_freq_idx[sb] = freq_index;
                }

                data.subband_done[sb] += 1;
                *total_tones += 1;
                progress = true;
            }
        }
        progress
    }

    fn fill_result_buf(&mut self, data: &[TChannelData]) {
        let mut used_contiguous_sb = [0u32; 2];
        let mut num_tones = [0u32; 2];

        for ch in 0..data.len() {
            let mut cur: i64 = -1;
            for (key, _) in data[ch].gha_infos.iter() {
                let sb = (key >> 10) as i64;
                if sb == cur + 1 {
                    used_contiguous_sb[ch] += 1;
                    num_tones[ch] += 1;
                    cur = sb;
                } else if sb == cur {
                    num_tones[ch] += 1;
                    continue;
                } else {
                    break;
                }
            }
        }

        let leader = (used_contiguous_sb[1] > used_contiguous_sb[0]) as usize;
        let follower = 1 - leader;

        // history envelope-stop snapshots (read before mutating result_buf)
        let hist0_env2: Vec<u32> = self.result_buf_history.waves[0]
            .wave_sb_infos
            .iter()
            .map(|i| i.envelope.1)
            .collect();
        let hist1_env2: Vec<u32> = self.result_buf_history.waves[1]
            .wave_sb_infos
            .iter()
            .map(|i| i.envelope.1)
            .collect();

        self.result_buf.second_is_leader = leader == 1;
        self.result_buf.num_tone_bands = used_contiguous_sb[leader] as u8;

        let stereo = data.len() == 2;

        if stereo {
            self.result_buf.waves[1].wave_params.clear();
            self.result_buf.waves[1].wave_sb_infos.clear();
            self.result_buf.waves[1]
                .wave_sb_infos
                .resize(used_contiguous_sb[leader] as usize, WaveSbInfo::default());
        }

        let leader_infos: Vec<(u32, GhaInfo)> = data[leader]
            .gha_infos
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        let follower_infos: Vec<(u32, GhaInfo)> = if stereo {
            data[follower]
                .gha_infos
                .iter()
                .map(|(k, v)| (*k, *v))
                .collect()
        } else {
            Vec::new()
        };
        let empty_env = [(At3PGhaData::INIT, At3PGhaData::INIT); SUBBANDS];
        let follower_env = if stereo {
            &data[follower].envelopes
        } else {
            &empty_env
        };
        let mut follower_idx = 0usize;

        self.result_buf.waves[0].wave_params.clear();
        self.result_buf.waves[0].wave_sb_infos.clear();
        self.result_buf.waves[0]
            .wave_sb_infos
            .resize(used_contiguous_sb[leader] as usize, WaveSbInfo::default());

        if used_contiguous_sb[leader] == 0 {
            return;
        }

        let mut prev_sb = 0u32;
        let mut index = 0u32;
        let used = used_contiguous_sb[leader];

        let mut it = 0usize;
        while it < leader_infos.len() {
            let (key, info) = leader_infos[it];
            let sb = key >> 10;
            if sb >= used {
                break;
            }
            let freq_index = key & 1023;
            let phase_index = gha_phase_to_index(info.phase);
            let amp_sf = amplitude_to_sf(info.magnitude);

            self.result_buf.waves[0].wave_sb_infos[sb as usize].wave_nums += 1;
            if sb != prev_sb {
                let hist_stop = hist0_env2
                    .get(prev_sb as usize)
                    .copied()
                    .unwrap_or(At3PGhaData::INIT);
                self.result_buf.waves[0].wave_sb_infos[prev_sb as usize].envelope =
                    adjust_envelope(data[leader].envelopes[prev_sb as usize], hist_stop);

                self.result_buf.waves[0].wave_sb_infos[sb as usize].wave_index = index as usize;

                if stereo {
                    self.fill_follower_res(
                        &data[leader].gha_infos,
                        &follower_infos,
                        &mut follower_idx,
                        follower_env,
                        prev_sb,
                        &hist1_env2,
                    );
                }
                prev_sb = sb;
            }
            self.result_buf.waves[0].wave_params.push(WaveParam {
                freq_index,
                amp_sf,
                amp_index: 1,
                phase_index,
            });
            it += 1;
            index += 1;
        }

        let hist_stop = hist0_env2
            .get(prev_sb as usize)
            .copied()
            .unwrap_or(At3PGhaData::INIT);
        self.result_buf.waves[0].wave_sb_infos[prev_sb as usize].envelope =
            adjust_envelope(data[leader].envelopes[prev_sb as usize], hist_stop);

        if stereo {
            self.fill_follower_res(
                &data[leader].gha_infos,
                &follower_infos,
                &mut follower_idx,
                follower_env,
                prev_sb,
                &hist1_env2,
            );
        }
    }

    fn fill_follower_res(
        &mut self,
        leader_map: &BTreeMap<u32, GhaInfo>,
        follower_infos: &[(u32, GhaInfo)],
        follower_idx: &mut usize,
        follower_envelopes: &[(u32, u32); SUBBANDS],
        cur_sb: u32,
        hist1_env2: &[u32],
    ) -> u32 {
        let hist_stop = hist1_env2
            .get(cur_sb as usize)
            .copied()
            .unwrap_or(At3PGhaData::INIT);

        let mut follower_sb_mode = 0u32;
        let mut next_sb = 0u32;
        let mut added = 0u32;

        while *follower_idx < follower_infos.len() {
            let (key, info) = follower_infos[*follower_idx];
            let sb = key >> 10;
            if sb > cur_sb {
                next_sb = sb;
                break;
            }
            follower_sb_mode |= (leader_map.get(&key).is_none() as u32) + 1;
            let freq_index = key & 1023;
            let phase_index = gha_phase_to_index(info.phase);
            let amp_sf = amplitude_to_sf(info.magnitude);
            self.result_buf.waves[1].wave_params.push(WaveParam {
                freq_index,
                amp_sf,
                amp_index: 1,
                phase_index,
            });
            *follower_idx += 1;
            added += 1;
        }

        match follower_sb_mode {
            0 => {
                self.result_buf.tone_sharing[cur_sb as usize] = false;
                self.result_buf.waves[1].wave_sb_infos[cur_sb as usize].wave_nums = 0;
            }
            1 => {
                self.result_buf.tone_sharing[cur_sb as usize] = true;
                let new_len = self.result_buf.waves[1].wave_params.len() - added as usize;
                self.result_buf.waves[1].wave_params.truncate(new_len);
            }
            _ => {
                self.result_buf.tone_sharing[cur_sb as usize] = false;
                let wi = self.result_buf.waves[1].wave_params.len() - added as usize;
                self.result_buf.waves[1].wave_sb_infos[cur_sb as usize].wave_index = wi;
                self.result_buf.waves[1].wave_sb_infos[cur_sb as usize].wave_nums = added as usize;
                self.result_buf.waves[1].wave_sb_infos[cur_sb as usize].envelope =
                    adjust_envelope(follower_envelopes[cur_sb as usize], hist_stop);
            }
        }
        next_sb
    }

    /// FFmpeg-driven residual filter (`ApplyFilter`). `d_present` indicates
    /// whether tonal data exists this frame; tone params are read from
    /// `self.result_buf`.
    fn apply_filter(&mut self, d_present: bool, b1: &mut [f32], b2: &mut [f32]) {
        let stereo = self.stereo;

        // memset current tones_info for both channels
        for ch in 0..2 {
            let cur = self.ch_unit.channels[ch].cur;
            self.ch_unit.channels[ch].tones_info_hist[cur] = Default::default();
        }

        if d_present {
            let wc = self.ch_unit.waves_cur;
            self.ch_unit.wave_synth_hist[wc].waves = [ff_dsp::WaveParam::default(); 48];
            self.ch_unit.wave_synth_hist[wc].num_tone_bands = self.result_buf.num_tone_bands as i32;
            self.ch_unit.wave_synth_hist[wc].tones_present = true;
            self.ch_unit.wave_synth_hist[wc].amplitude_mode = 1;
            self.ch_unit.wave_synth_hist[wc].tones_index = 0;
        } else {
            let wc = self.ch_unit.waves_cur;
            self.ch_unit.wave_synth_hist[wc].tones_present = false;
        }

        if d_present {
            let d = &self.result_buf;
            let num_tone_bands =
                self.ch_unit.wave_synth_hist[self.ch_unit.waves_cur].num_tone_bands;
            let nch = if stereo { 2 } else { 1 };
            for ch in 0..nch {
                let cur = self.ch_unit.channels[ch].cur;
                for i in 0..num_tone_bands as usize {
                    if ch != 0 && d.tone_sharing[i] {
                        continue;
                    }
                    let num_wavs = d.num_waves(ch, i);
                    self.ch_unit.channels[ch].tones_info_hist[cur][i].num_wavs = num_wavs as i32;

                    let envelope = d.envelope(ch, i);
                    let ti = &mut self.ch_unit.channels[ch].tones_info_hist[cur][i];
                    if envelope.0 != At3PGhaData::EMPTY_POINT {
                        ti.pend_env.has_start_point = 1;
                        ti.pend_env.start_pos = envelope.0 as i32;
                    } else {
                        ti.pend_env.has_start_point = 0;
                        ti.pend_env.start_pos = -1;
                    }
                    if envelope.1 != At3PGhaData::EMPTY_POINT {
                        ti.pend_env.has_stop_point = 1;
                        ti.pend_env.stop_pos = envelope.1 as i32;
                    } else {
                        ti.pend_env.has_stop_point = 0;
                        ti.pend_env.stop_pos = 32;
                    }
                }

                for sb in 0..num_tone_bands as usize {
                    if d.num_waves(ch, sb) != 0 {
                        let wc = self.ch_unit.waves_cur;
                        let cur = self.ch_unit.channels[ch].cur;
                        let start = self.ch_unit.wave_synth_hist[wc].tones_index;
                        let nw = self.ch_unit.channels[ch].tones_info_hist[cur][sb].num_wavs;
                        if start + nw > 48 {
                            panic!("too many tones: {}", start + nw);
                        }
                        self.ch_unit.channels[ch].tones_info_hist[cur][sb].start_index = start;
                        self.ch_unit.wave_synth_hist[wc].tones_index += nw;
                    }
                }

                for sb in 0..num_tone_bands as usize {
                    if d.num_waves(ch, sb) != 0 {
                        let cur = self.ch_unit.channels[ch].cur;
                        let start =
                            self.ch_unit.channels[ch].tones_info_hist[cur][sb].start_index as usize;
                        let (w, wnum) = d.waves(ch, sb);
                        self.ch_unit.channels[ch].tones_info_hist[cur][sb].num_wavs = wnum as i32;
                        let wc = self.ch_unit.waves_cur;
                        for j in 0..wnum {
                            let iw = &mut self.ch_unit.wave_synth_hist[wc].waves[start + j];
                            iw.freq_index = w[j].freq_index as i32;
                            iw.amp_index = w[j].amp_index as i32;
                            iw.amp_sf = w[j].amp_sf as i32;
                            iw.phase_index = w[j].phase_index as i32;
                        }
                    }
                }
            }

            if stereo {
                for i in 0..num_tone_bands as usize {
                    if d.tone_sharing[i] {
                        let cur0 = self.ch_unit.channels[0].cur;
                        let src = self.ch_unit.channels[0].tones_info_hist[cur0][i];
                        let cur1 = self.ch_unit.channels[1].cur;
                        self.ch_unit.channels[1].tones_info_hist[cur1][i] = src;
                    }
                    if d.second_is_leader {
                        let cur0 = self.ch_unit.channels[0].cur;
                        let cur1 = self.ch_unit.channels[1].cur;
                        let a = self.ch_unit.channels[0].tones_info_hist[cur0][i];
                        let b = self.ch_unit.channels[1].tones_info_hist[cur1][i];
                        self.ch_unit.channels[0].tones_info_hist[cur0][i] = b;
                        self.ch_unit.channels[1].tones_info_hist[cur1][i] = a;
                    }
                }
            }
        }

        let nch = if stereo { 2 } else { 1 };
        for ch in 0..nch {
            let x: &mut [f32] = if ch == 0 { b1 } else { b2 };
            let cur = self.ch_unit.channels[ch].cur;
            let prev = self.ch_unit.channels[ch].prev;
            let tones_present = self.ch_unit.wave_synth_hist[self.ch_unit.waves_cur].tones_present
                || self.ch_unit.wave_synth_hist[self.ch_unit.waves_prev].tones_present;
            if tones_present {
                for sb in 0..SUBBANDS {
                    let nw_cur = self.ch_unit.channels[ch].tones_info_hist[cur][sb].num_wavs;
                    let nw_prev = self.ch_unit.channels[ch].tones_info_hist[prev][sb].num_wavs;
                    if nw_cur != 0 || nw_prev != 0 {
                        self.ch_unit
                            .generate_tones(ch, sb, &mut x[sb * 128..sb * 128 + 128]);
                    }
                }
            }
        }

        // swap cur/prev per channel and for waves
        for ch in 0..2 {
            let c = self.ch_unit.channels[ch].cur;
            self.ch_unit.channels[ch].cur = self.ch_unit.channels[ch].prev;
            self.ch_unit.channels[ch].prev = c;
        }
        std::mem::swap(&mut self.ch_unit.waves_cur, &mut self.ch_unit.waves_prev);
    }
}

impl GhaProcessor for TGhaProcessor {
    fn do_analyze(
        &mut self,
        b1: [&[f32]; 2],
        b2: [&[f32]; 2],
        w1: &mut [f32],
        w2: &mut [f32],
    ) -> Option<&At3PGhaData> {
        let nch = if self.stereo { 2 } else { 1 };
        let mut data: Vec<TChannelData> = (0..nch).map(|_| TChannelData::new()).collect();

        let src_cur: [&[f32]; 2] = [b1[0], b2[0]];
        let src_next: [&[f32]; 2] = [b1[1], b2[1]];

        for ch in 0..nch {
            for sb in 0..SUBBANDS {
                let dst = &mut data[ch].buf[sb * GHA_SUBBAND_BUF_SZ..];
                dst[..SAMPLES_PER_SUBBAND].copy_from_slice(
                    &src_cur[ch]
                        [sb * SAMPLES_PER_SUBBAND..sb * SAMPLES_PER_SUBBAND + SAMPLES_PER_SUBBAND],
                );
                dst[SAMPLES_PER_SUBBAND..SAMPLES_PER_SUBBAND + LOOK_AHEAD].copy_from_slice(
                    &src_next[ch][sb * SAMPLES_PER_SUBBAND..sb * SAMPLES_PER_SUBBAND + LOOK_AHEAD],
                );
            }
        }

        let mut total_tones = 0usize;
        loop {
            let mut progress = [false; 2];
            for ch in 0..nch {
                let (cur, next) = (src_cur[ch], src_next[ch]);
                progress[ch] = self.do_round(&mut data[ch], cur, next, &mut total_tones);
            }
            if !((progress[0] || progress[1]) && total_tones < 48) {
                break;
            }
        }

        if total_tones == 0 {
            self.apply_filter(false, w1, w2);
            return None;
        }

        self.fill_result_buf(&data);
        self.result_buf_history = self.result_buf.clone();
        self.apply_filter(true, w1, w2);

        Some(&self.result_buf)
    }
}
