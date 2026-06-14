//! FFmpeg-derived ATRAC3+ tone synthesis (used by the GHA analyzer to compute
//! the tonal residual).
//!
//! Port of `atracdenc/src/atrac/at3p/ff/atrac3plusdsp.c` and the relevant
//! structs from `ff/atrac3plus.h` (FFmpeg, LGPL-2.1-or-later). Only the
//! encoder-used subset of fields is modeled. C "pointers" into the 2-frame
//! history arrays are represented with `cur`/`prev` index fields.

use std::sync::LazyLock;

pub const SUBBANDS: usize = 16;

static SINE_TABLE: LazyLock<[f32; 2048]> = LazyLock::new(|| {
    let mut t = [0.0f32; 2048];
    for i in 0..2048 {
        t[i] = (std::f64::consts::TAU * i as f64 / 2048.0).sin() as f32;
    }
    t
});

static HANN_WINDOW: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut w = [0.0f32; 256];
    for i in 0..256 {
        w[i] = ((1.0f64 - (std::f64::consts::TAU * i as f64 / 256.0).cos()) * 0.5) as f32;
    }
    w
});

/// `amp_sf_tab[i] = exp2f((i - 3) / 4)`.
pub static AMP_SF_TAB: LazyLock<[f32; 64]> = LazyLock::new(|| {
    let mut t = [0.0f32; 64];
    for i in 0..64 {
        t[i] = ((i as f32 - 3.0) / 4.0).exp2();
    }
    t
});

#[inline]
fn dequant_phase(ph: i32) -> i32 {
    (ph & 0x1F) << 6
}

/// Amplitude envelope of a group of sine waves.
#[derive(Debug, Clone, Copy, Default)]
pub struct WaveEnvelope {
    pub has_start_point: i32,
    pub has_stop_point: i32,
    pub start_pos: i32,
    pub stop_pos: i32,
}

/// Parameters of a group of sine waves.
#[derive(Debug, Clone, Copy, Default)]
pub struct WavesData {
    pub pend_env: WaveEnvelope,
    pub curr_env: WaveEnvelope,
    pub num_wavs: i32,
    pub start_index: i32,
}

/// Parameters of a single sine wave.
#[derive(Debug, Clone, Copy, Default)]
pub struct WaveParam {
    pub freq_index: i32,
    pub amp_sf: i32,
    pub amp_index: i32,
    pub phase_index: i32,
}

/// Per-unit sine wave parameters.
#[derive(Clone)]
pub struct WaveSynthParams {
    pub tones_present: bool,
    pub amplitude_mode: i32,
    pub num_tone_bands: i32,
    pub invert_phase: [u8; SUBBANDS],
    pub tones_index: i32,
    pub waves: [WaveParam; 48],
}

impl Default for WaveSynthParams {
    fn default() -> Self {
        Self {
            tones_present: false,
            amplitude_mode: 0,
            num_tone_bands: 0,
            invert_phase: [0; SUBBANDS],
            tones_index: 0,
            waves: [WaveParam::default(); 48],
        }
    }
}

/// Sound channel parameters: 2-frame tones history with cur/prev selectors.
#[derive(Clone)]
pub struct ChanParams {
    pub tones_info_hist: [[WavesData; SUBBANDS]; 2],
    /// Index (0/1) of the "current frame" row (C `tones_info`).
    pub cur: usize,
    /// Index (0/1) of the "previous frame" row (C `tones_info_prev`).
    pub prev: usize,
}

impl Default for ChanParams {
    fn default() -> Self {
        Self {
            tones_info_hist: [[WavesData::default(); SUBBANDS]; 2],
            cur: 0,
            prev: 1,
        }
    }
}

/// Channel unit context.
#[derive(Clone)]
pub struct ChanUnitCtx {
    pub channels: [ChanParams; 2],
    pub wave_synth_hist: [WaveSynthParams; 2],
    /// Index (0/1) of `waves_info` (current frame).
    pub waves_cur: usize,
    /// Index (0/1) of `waves_info_prev`.
    pub waves_prev: usize,
}

impl Default for ChanUnitCtx {
    fn default() -> Self {
        let mut ctx = Self {
            channels: [ChanParams::default(), ChanParams::default()],
            wave_synth_hist: [WaveSynthParams::default(), WaveSynthParams::default()],
            waves_cur: 0,
            waves_prev: 1,
        };
        for ch in 0..2 {
            ctx.channels[ch].cur = 0;
            ctx.channels[ch].prev = 1;
        }
        ctx
    }
}

/// Synthesize sine waves according to given parameters (FFmpeg `waves_synth`).
fn waves_synth(
    amplitude_mode: i32,
    waves: &[WaveParam],
    wave: &WavesData,
    envelope: &WaveEnvelope,
    invert_phase: bool,
    reg_offset: i32,
    out: &mut [f32; 128],
) {
    let sine_table = &*SINE_TABLE;
    let hann_window = &*HANN_WINDOW;
    let amp_sf_tab = &*AMP_SF_TAB;

    let start = wave.start_index as usize;
    for wn in 0..wave.num_wavs as usize {
        let wp = &waves[start + wn];
        let amp = amp_sf_tab[wp.amp_sf as usize] as f64
            * if amplitude_mode == 0 {
                (wp.amp_index + 1) as f64 / 15.13f32 as f64
            } else {
                1.0
            };

        let inc = wp.freq_index;
        let mut pos = (dequant_phase(wp.phase_index) - (reg_offset ^ 128) * inc) & 2047;

        for i in 0..128 {
            out[i] = (out[i] as f64 + sine_table[pos as usize] as f64 * amp) as f32;
            pos = (pos + inc) & 2047;
        }
    }

    if invert_phase {
        for v in out.iter_mut() {
            *v *= -1.0;
        }
    }

    if envelope.has_start_point != 0 {
        let pos = (envelope.start_pos << 2) - reg_offset;
        if pos > 0 && pos <= 128 {
            let pos = pos as usize;
            for v in out[..pos].iter_mut() {
                *v = 0.0;
            }
            if envelope.has_stop_point == 0 || envelope.start_pos != envelope.stop_pos {
                out[pos] *= hann_window[0];
                out[pos + 1] *= hann_window[32];
                out[pos + 2] *= hann_window[64];
                out[pos + 3] *= hann_window[96];
            }
        }
    }

    if envelope.has_stop_point != 0 {
        let pos = ((envelope.stop_pos + 1) << 2) - reg_offset;
        if pos > 0 && pos <= 128 {
            let pos = pos as usize;
            out[pos - 4] *= hann_window[96];
            out[pos - 3] *= hann_window[64];
            out[pos - 2] *= hann_window[32];
            out[pos - 1] *= hann_window[0];
            for v in out[pos..128].iter_mut() {
                *v = 0.0;
            }
        }
    }
}

fn vector_fmul(dst: &mut [f32; 128], src1: &[f32]) {
    for i in 0..128 {
        dst[i] *= src1[i];
    }
}

impl ChanUnitCtx {
    /// Generate tones for a subband and subtract them from `out` (FFmpeg
    /// `ff_atrac3p_generate_tones`). `out` is the 128-sample subband residual.
    pub fn generate_tones(&mut self, ch_num: usize, sb: usize, out: &mut [f32]) {
        let mut wavreg1 = [0.0f32; 128];
        let mut wavreg2 = [0.0f32; 128];

        let cur = self.channels[ch_num].cur;
        let prev = self.channels[ch_num].prev;

        // tones_now = tones_info_prev[sb] (read-only snapshot)
        let tones_now = self.channels[ch_num].tones_info_hist[prev][sb];

        // Reconstruct full envelopes for the "next" group (writes curr_env).
        let tones_next_pend = self.channels[ch_num].tones_info_hist[cur][sb].pend_env;
        let mut curr_env = WaveEnvelope::default();

        if tones_next_pend.has_start_point != 0
            && tones_next_pend.start_pos < tones_next_pend.stop_pos
        {
            curr_env.has_start_point = 1;
            curr_env.start_pos = tones_next_pend.start_pos + 32;
        } else if tones_now.pend_env.has_start_point != 0 {
            curr_env.has_start_point = 1;
            curr_env.start_pos = tones_now.pend_env.start_pos;
        } else {
            curr_env.has_start_point = 0;
            curr_env.start_pos = 0;
        }

        if tones_now.pend_env.has_stop_point != 0
            && tones_now.pend_env.stop_pos >= curr_env.start_pos
        {
            curr_env.has_stop_point = 1;
            curr_env.stop_pos = tones_now.pend_env.stop_pos;
        } else if tones_next_pend.has_stop_point != 0 {
            curr_env.has_stop_point = 1;
            curr_env.stop_pos = tones_next_pend.stop_pos + 32;
        } else {
            curr_env.has_stop_point = 0;
            curr_env.stop_pos = 64;
        }

        self.channels[ch_num].tones_info_hist[cur][sb].curr_env = curr_env;
        let tones_next = self.channels[ch_num].tones_info_hist[cur][sb];

        let reg1_env_nonzero = if tones_now.curr_env.stop_pos < 32 {
            0
        } else {
            1
        };
        let reg2_env_nonzero = if curr_env.start_pos >= 32 { 0 } else { 1 };

        let inv_prev = self.wave_synth_hist[self.waves_prev].invert_phase[sb] as usize & ch_num;
        let inv_cur = self.wave_synth_hist[self.waves_cur].invert_phase[sb] as usize & ch_num;

        if tones_now.num_wavs != 0 && reg1_env_nonzero != 0 {
            let amplitude_mode = self.wave_synth_hist[self.waves_prev].amplitude_mode;
            let waves = self.wave_synth_hist[self.waves_prev].waves;
            waves_synth(
                amplitude_mode,
                &waves,
                &tones_now,
                &tones_now.curr_env,
                inv_prev != 0,
                128,
                &mut wavreg1,
            );
        }

        if tones_next.num_wavs != 0 && reg2_env_nonzero != 0 {
            let amplitude_mode = self.wave_synth_hist[self.waves_cur].amplitude_mode;
            let waves = self.wave_synth_hist[self.waves_cur].waves;
            waves_synth(
                amplitude_mode,
                &waves,
                &tones_next,
                &curr_env,
                inv_cur != 0,
                0,
                &mut wavreg2,
            );
        }

        let hann_window = &*HANN_WINDOW;
        if tones_now.num_wavs != 0
            && tones_next.num_wavs != 0
            && reg1_env_nonzero != 0
            && reg2_env_nonzero != 0
        {
            vector_fmul(&mut wavreg1, &hann_window[128..]);
            vector_fmul(&mut wavreg2, &hann_window[..]);
        } else {
            if tones_now.num_wavs != 0 && tones_now.curr_env.has_stop_point == 0 {
                vector_fmul(&mut wavreg1, &hann_window[128..]);
            }
            if tones_next.num_wavs != 0 && curr_env.has_start_point == 0 {
                vector_fmul(&mut wavreg2, &hann_window[..]);
            }
        }

        for i in 0..128 {
            out[i] -= wavreg1[i] + wavreg2[i];
        }
    }
}
