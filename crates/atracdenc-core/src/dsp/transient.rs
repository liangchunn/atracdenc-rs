use std::io::Write;

use crate::util::get_first_set_bit;

const PREV_BUF_SZ: usize = 20;
const FIR_LEN: usize = 21;

fn calculate_rms(input: &[f32]) -> f32 {
    (input.iter().map(|x| x * x).sum::<f32>() / input.len() as f32).sqrt()
}

fn calculate_peak(input: &[f32]) -> f32 {
    input.iter().map(|x| x.abs()).fold(0.0, f32::max)
}

#[derive(Debug, Clone)]
pub struct TransientDetector {
    short_sz: usize,
    block_sz: usize,
    n_short_blocks: usize,
    hpf_buffer: Vec<f32>,
    last_energy: f32,
    last_transient_pos: u16,
}

impl TransientDetector {
    pub fn new(short_sz: u16, block_sz: u16) -> Self {
        let short_sz = short_sz as usize;
        let block_sz = block_sz as usize;
        Self {
            short_sz,
            block_sz,
            n_short_blocks: block_sz / short_sz,
            hpf_buffer: vec![0.0; block_sz + FIR_LEN],
            last_energy: 0.0,
            last_transient_pos: 0,
        }
    }

    fn hp_filter(&mut self, input: &[f32], out: &mut [f32]) {
        assert_eq!(self.block_sz, input.len());
        assert_eq!(self.block_sz, out.len());
        const FIR_COEF: [f32; 10] = [
            -8.65163e-18 * 2.0,
            -0.00851586 * 2.0,
            -6.74764e-18 * 2.0,
            0.0209036 * 2.0,
            -3.36639e-17 * 2.0,
            -0.0438162 * 2.0,
            -1.54175e-17 * 2.0,
            0.0931738 * 2.0,
            -5.52212e-17 * 2.0,
            -0.313819 * 2.0,
        ];

        self.hpf_buffer[PREV_BUF_SZ..PREV_BUF_SZ + self.block_sz].copy_from_slice(input);
        for (i, y) in out.iter_mut().enumerate() {
            let mut s = self.hpf_buffer[i + 10];
            let mut s2 = 0.0;
            for j in (0..(((FIR_LEN - 1) / 2) - 1)).step_by(2) {
                s += FIR_COEF[j] * (self.hpf_buffer[i + j] + self.hpf_buffer[i + FIR_LEN - j]);
                s2 += FIR_COEF[j + 1]
                    * (self.hpf_buffer[i + j + 1] + self.hpf_buffer[i + FIR_LEN - j - 1]);
            }
            *y = (s + s2) / 2.0;
        }
        self.hpf_buffer[..PREV_BUF_SZ]
            .copy_from_slice(&input[self.block_sz - PREV_BUF_SZ..self.block_sz]);
    }

    pub fn detect(&mut self, buf: &[f32]) -> bool {
        assert_eq!(self.block_sz, buf.len());
        let n_blocks_to_analyze = self.n_short_blocks + 1;
        let mut rms_per_short_block = vec![0.0; n_blocks_to_analyze];
        let mut filtered = vec![0.0; self.block_sz];
        self.hp_filter(buf, &mut filtered);

        let mut trans = false;
        rms_per_short_block[0] = self.last_energy;
        for i in 1..n_blocks_to_analyze {
            rms_per_short_block[i] =
                19.0 * calculate_rms(&filtered[(i - 1) * self.short_sz..i * self.short_sz]).log10();
            if rms_per_short_block[i] - rms_per_short_block[i - 1] > 16.0 {
                trans = true;
                self.last_transient_pos = i as u16;
            }
            if rms_per_short_block[i - 1] - rms_per_short_block[i] > 20.0 {
                trans = true;
                self.last_transient_pos = i as u16;
            }
        }
        self.last_energy = rms_per_short_block[self.n_short_blocks];
        trans
    }

    pub fn last_transient_pos(&self) -> u32 {
        self.last_transient_pos as u32
    }
}

pub fn analyze_gain(
    input: &[f32],
    max_points: u32,
    use_rms: bool,
    mut subframe_low: Option<&mut Vec<f32>>,
    mut subframe_high: Option<&mut Vec<f32>>,
) -> Vec<f32> {
    let mut res = Vec::new();
    let step = input.len() / max_points as usize;

    if let Some(low) = subframe_low.as_deref_mut() {
        low.clear();
        low.reserve(max_points as usize);
    }
    if let Some(high) = subframe_high.as_deref_mut() {
        high.clear();
        high.reserve(max_points as usize);
    }

    for pos in (0..input.len()).step_by(step) {
        let chunk = &input[pos..pos + step];
        let val = if use_rms {
            calculate_rms(chunk)
        } else {
            calculate_peak(chunk)
        };
        res.push(val);

        if subframe_low.is_some() || subframe_high.is_some() {
            let chunks = 8;
            let chunk_sz = std::cmp::max(1, step / chunks);
            let mut micro = Vec::with_capacity(step.div_ceil(chunk_sz));
            for off in (0..step).step_by(chunk_sz) {
                let n = std::cmp::min(chunk_sz, step - off);
                let micro_chunk = &input[pos + off..pos + off + n];
                micro.push(if use_rms {
                    calculate_rms(micro_chunk)
                } else {
                    calculate_peak(micro_chunk)
                });
            }
            micro.sort_by(|a, b| a.total_cmp(b));
            let lo_idx = micro.len() / 4;
            let hi_idx = (micro.len() * 3) / 4;
            if let Some(low) = subframe_low.as_deref_mut() {
                low.push(micro[lo_idx]);
            }
            if let Some(high) = subframe_high.as_deref_mut() {
                high.push(micro[hi_idx]);
            }
        }
    }

    res
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct GainCurvePoint {
    pub level: u32,
    pub location: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CurveBuilderCtx {
    pub last_level: f32,
    pub last_hpf_energy: f32,
    pub last_target: f32,
}

#[derive(Debug, Clone, Copy)]
struct PlateauResult {
    level: f32,
    max_raw: f32,
    release_at_end: bool,
}

fn relation_to_idx(mut x: f32) -> u16 {
    if x <= 0.5 {
        x = 1.0 / x.max(0.000_488_281_25);
        4 + get_first_set_bit(x as u32)
    } else {
        x = x.min(16.0);
        4 - get_first_set_bit(x as u32)
    }
}

fn median_filter<const RADIUS: usize>(input: &[f32], out: &mut [f32]) {
    assert_eq!(input.len(), out.len());
    let n = input.len() as isize;
    let mut w = vec![0.0; RADIUS * 2 + 1];

    for (i, out_val) in out.iter_mut().enumerate() {
        let lo = 0.max(i as isize - RADIUS as isize);
        let hi = (n - 1).min(i as isize + RADIUS as isize);
        let mut wn = 0;
        for j in lo..=hi {
            w[wn] = input[j as usize];
            wn += 1;
        }
        w[..wn].sort_by(|a, b| a.total_cmp(b));
        *out_val = w[wn / 2];
    }
}

fn find_plateau(input: &[f32], min_contiguous: usize) -> PlateauResult {
    let n = input.len();
    let max_raw = input.iter().copied().fold(0.0, f32::max);
    if n < min_contiguous {
        return PlateauResult {
            level: 0.0,
            max_raw,
            release_at_end: false,
        };
    }

    let mut filtered = vec![0.0; n];
    median_filter::<1>(input, &mut filtered);

    let mut best_level = 0.0;
    let mut best_end = None;
    for j in 0..=n - min_contiguous {
        let min_val = filtered[j..j + min_contiguous]
            .iter()
            .copied()
            .fold(filtered[j], f32::min);
        if min_val > best_level {
            best_level = min_val;
            best_end = Some(j + min_contiguous - 1);
        }
    }

    let Some(mut best_end) = best_end else {
        return PlateauResult {
            level: 0.0,
            max_raw,
            release_at_end: false,
        };
    };

    if best_level < 1.0e-6 {
        return PlateauResult {
            level: 0.0,
            max_raw,
            release_at_end: false,
        };
    }

    while best_end + 1 < n && filtered[best_end + 1] >= best_level {
        best_end += 1;
    }

    let mut release_at_end = false;
    if best_end < n - 1 {
        if input[n - 1] < best_level * 0.1 {
            release_at_end = true;
        } else {
            let any_high_after = input[best_end + 1..].iter().any(|v| *v >= best_level * 0.7);
            release_at_end = !any_high_after && input[n - 1] < best_level * 0.5;
        }
    }

    PlateauResult {
        level: best_level,
        max_raw,
        release_at_end,
    }
}

fn boundary_transient_score(env: &[f32], loc: usize, win: usize) -> f32 {
    assert!(loc > 0 && loc < env.len());
    let left_start = loc.saturating_sub(win);
    let left_max = env[left_start..loc].iter().copied().fold(0.0, f32::max);
    let right_end = env.len().min(loc + win);
    let right_max = env[loc..right_end].iter().copied().fold(0.0, f32::max);

    let attack = (right_max + 1.0e-9) / (left_max + 1.0e-9);
    let release = (left_max + 1.0e-9) / (right_max + 1.0e-9);
    attack.max(release)
}

pub fn calc_curve(
    input: &[f32],
    ctx: &mut CurveBuilderCtx,
    _next_level: Option<f32>,
    min_score: f32,
    mut yaml_log: Option<&mut (dyn Write + '_)>,
    subframe_low: Option<&[f32]>,
    subframe_high: Option<&[f32]>,
) -> Vec<GainCurvePoint> {
    let mut curve = Vec::new();
    if input.is_empty() {
        return curve;
    }

    let plateau = find_plateau(input, 3);
    let use_plateau =
        plateau.level > 1.0e-6 && !plateau.release_at_end && plateau.level >= plateau.max_raw * 0.4;
    let target = if use_plateau {
        plateau.level
    } else {
        *input.last().unwrap()
    };

    if let Some(log) = yaml_log.as_deref_mut() {
        let _ = writeln!(log, "        plateau_level: {:.6}", plateau.level);
        let _ = writeln!(log, "        plateau_max_raw: {:.6}", plateau.max_raw);
        let _ = writeln!(
            log,
            "        plateau_release: {}",
            if plateau.release_at_end {
                "true"
            } else {
                "false"
            }
        );
        let _ = writeln!(
            log,
            "        target: {:.6}  # source: {}",
            target,
            if use_plateau { "plateau" } else { "in.back" }
        );
    }

    let saved_last_level = ctx.last_level;
    let saved_last_target = ctx.last_target;
    ctx.last_level = *input.last().unwrap();
    ctx.last_target = target;

    if target < 1.0e-6 || saved_last_level < 1.0e-6 {
        return curve;
    }

    let n = input.len();
    let mut filtered = vec![0.0; n];
    median_filter::<1>(input, &mut filtered);

    let max_gain = input.iter().copied().fold(0.0, f32::max);
    let intra_ratio = max_gain / target.max(1.0e-9);
    let mut inter_ratio = 1.0;
    if saved_last_target > 1.0e-6 {
        inter_ratio = saved_last_target.max(target) / saved_last_target.min(target).max(1.0e-9);
    }
    let sticky_frame_eligible = subframe_low.is_some_and(|low| low.len() == n)
        && subframe_high.is_some_and(|high| high.len() == n)
        && intra_ratio <= 7.0
        && inter_ratio <= 10.0;

    if let Some(log) = yaml_log.as_deref_mut() {
        let _ = writeln!(
            log,
            "        sticky_frame_eligible: {}",
            if sticky_frame_eligible {
                "true"
            } else {
                "false"
            }
        );
        let _ = writeln!(log, "        sticky_intra_ratio: {:.4}", intra_ratio);
        let _ = writeln!(log, "        sticky_inter_ratio: {:.4}", inter_ratio);
    }

    let mut sf_level = vec![0_u16; n];
    for i in 0..n {
        let ratio_center = filtered[i] / target;
        let mut level = relation_to_idx(ratio_center);
        if i > 0 && sticky_frame_eligible {
            let low = subframe_low.unwrap();
            let high = subframe_high.unwrap();
            let mut ratio_lo = low[i] / target;
            let mut ratio_hi = high[i] / target;
            if ratio_lo > ratio_hi {
                std::mem::swap(&mut ratio_lo, &mut ratio_hi);
            }
            let idx_lo = relation_to_idx(ratio_lo);
            let idx_hi = relation_to_idx(ratio_hi);
            let min_idx = idx_lo.min(idx_hi);
            let max_idx = idx_lo.max(idx_hi);
            let prev = sf_level[i - 1];
            let idx_span = max_idx - min_idx;
            if idx_span <= 1
                && (level as i32 - prev as i32).abs() == 1
                && prev >= min_idx
                && prev <= max_idx
            {
                level = prev;
            }
        }
        sf_level[i] = level;
    }

    let mut target_sf = 0;
    for sf in (0..n - 1).rev() {
        if sf_level[sf] != 4 {
            target_sf = sf + 1;
            break;
        }
    }

    if target_sf == 0 {
        return curve;
    }

    let mut boundary_score = vec![1.0; n + 1];
    for (loc, score) in boundary_score
        .iter_mut()
        .enumerate()
        .take(target_sf + 1)
        .skip(1)
    {
        *score = boundary_transient_score(&filtered, loc, 3);
    }

    if let Some(log) = yaml_log.as_deref_mut() {
        let _ = writeln!(log, "        transient_min_score: {:.4}", min_score);
        let _ = writeln!(log, "        transient_window: 3");
    }

    #[derive(Clone)]
    struct Transition {
        loc: usize,
        level: u16,
        delta: i32,
    }

    let mut trans = Vec::new();
    let mut prev = 4_u16;
    for sf in (0..target_sf).rev() {
        let lev = sf_level[sf];
        if lev != prev {
            let loc = sf + 1;
            let delta = (lev as i32 - prev as i32).abs();
            let score = boundary_score[loc];
            let keep = loc == target_sf || delta >= 2 || score >= min_score;
            if keep {
                trans.push(Transition {
                    loc,
                    level: lev,
                    delta,
                });
                prev = lev;
            } else if let Some(log) = yaml_log.as_deref_mut() {
                let _ = writeln!(
                    log,
                    "        transition_pruned: {{loc: {loc}, delta: {delta}, score: {score:.4}}}"
                );
            }
        }
    }
    trans.reverse();

    if trans.len() > 6 {
        trans.sort_by(|a, b| b.delta.cmp(&a.delta).then_with(|| b.loc.cmp(&a.loc)));
        trans.truncate(6);
        trans.sort_by_key(|t| t.loc);
    }

    curve.extend(trans.into_iter().map(|t| GainCurvePoint {
        level: t.level as u32,
        location: t.loc as u32,
    }));
    curve
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_gain_simple_peak_case() {
        let mut input = vec![0.0; 256];
        for (i, sample) in input.iter_mut().enumerate() {
            *sample = if i <= 24 {
                1.0
            } else if i <= 32 {
                8.0
            } else if i <= 66 {
                128.0
            } else {
                0.5
            };
        }

        let res = analyze_gain(&input, 32, false, None, None);
        assert_eq!(32, res.len());
        for v in &res[0..3] {
            assert_eq!(1.0, *v);
        }
        assert_eq!(8.0, res[3]);
        for v in &res[4..9] {
            assert_eq!(128.0, *v);
        }
        for v in &res[9..32] {
            assert_eq!(0.5, *v);
        }
    }

    #[test]
    fn transient_detector_flags_large_energy_change() {
        let mut detector = TransientDetector::new(32, 256);
        let mut quiet = vec![0.001; 256];
        let _ = detector.detect(&quiet);
        assert!(!detector.detect(&quiet));
        quiet[128..].fill(10.0);
        assert!(detector.detect(&quiet));
        assert!(detector.last_transient_pos() > 0);
    }

    #[test]
    fn calc_curve_skips_first_frame_then_emits_reasonable_points() {
        let gain = [1.0, 1.0, 1.0, 8.0, 8.0, 8.0, 8.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let mut ctx = CurveBuilderCtx::default();
        assert!(calc_curve(&gain, &mut ctx, None, 2.0, None, None, None).is_empty());

        let curve = calc_curve(&gain, &mut ctx, None, 2.0, None, None, None);
        assert!(curve.len() <= 6);
        assert!(curve.iter().all(|p| p.level < 16 && p.location < 32));
        assert!(curve.windows(2).all(|w| w[0].location <= w[1].location));
    }
}
