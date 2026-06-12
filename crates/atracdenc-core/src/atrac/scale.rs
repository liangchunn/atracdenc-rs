use crate::util::to_int;

const MAX_SCALE: f32 = 1.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ScaledBlock {
    pub scale_factor_index: u8,
    pub values: Vec<f32>,
    pub energy: f32,
}

#[derive(Debug, Clone)]
pub struct BlockLayout<'a> {
    pub num_qmf: usize,
    pub blocks_per_band: &'a [u8],
    pub specs_start_short: &'a [u16],
    pub specs_start_long: &'a [u16],
    pub specs_per_block: &'a [u16],
    pub short_windows: &'a [bool],
}

#[derive(Debug, Clone)]
pub struct Scaler {
    scale_index: Vec<(f32, u8)>,
}

impl Scaler {
    pub fn new(scale_table: &[f32]) -> Self {
        let mut scale_index = scale_table
            .iter()
            .enumerate()
            .map(|(idx, scale)| (*scale, idx as u8))
            .collect::<Vec<_>>();
        scale_index.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { scale_index }
    }

    pub fn scale(&self, input: &[f32]) -> ScaledBlock {
        let mut max_abs_spec = input.iter().map(|x| x.abs()).fold(0.0, f32::max);
        if max_abs_spec > MAX_SCALE {
            max_abs_spec = MAX_SCALE;
        }

        let pos = self
            .scale_index
            .partition_point(|(scale, _)| *scale < max_abs_spec);
        let (scale_factor, scale_factor_index) =
            self.scale_index[pos.min(self.scale_index.len() - 1)];

        let mut res = ScaledBlock {
            scale_factor_index,
            values: Vec::with_capacity(input.len()),
            energy: 0.0,
        };

        for x in input {
            let mut scaled_value = *x / scale_factor;
            res.energy += x * x;
            if scaled_value.abs() >= 1.0 {
                scaled_value = if scaled_value > 0.0 {
                    0.99999
                } else {
                    -0.99999
                };
            }
            res.values.push(scaled_value);
        }

        res
    }

    pub fn scale_frame(&self, specs: &[f32], layout: &BlockLayout<'_>) -> Vec<ScaledBlock> {
        let max_bfus = layout.specs_per_block.len();
        let mut scaled_blocks = Vec::with_capacity(max_bfus);
        for band_num in 0..layout.num_qmf {
            let short_win_mode = layout.short_windows[band_num];
            for block_num in layout.blocks_per_band[band_num]..layout.blocks_per_band[band_num + 1]
            {
                let block_num = block_num as usize;
                let spec_num_start = if short_win_mode {
                    layout.specs_start_short[block_num]
                } else {
                    layout.specs_start_long[block_num]
                } as usize;
                let len = layout.specs_per_block[block_num] as usize;
                scaled_blocks.push(self.scale(&specs[spec_num_start..spec_num_start + len]));
            }
        }
        scaled_blocks
    }
}

pub fn quant_mantissas(
    input: &[f32],
    first: u32,
    last: u32,
    mul: f32,
    ea: bool,
    mantissas: &mut [i32],
) -> f32 {
    let first = first as usize;
    let last = last as usize;
    assert!(last <= mantissas.len());
    assert!(last - first <= input.len());

    let mut e1 = 0.0;
    let mut e2 = 0.0;
    let inv2 = 1.0 / (mul * mul);

    if !ea {
        for (j, f) in (first..last).enumerate() {
            let t = input[j] * mul;
            e1 += input[j] * input[j];
            mantissas[f] = to_int(t);
            e2 += mantissas[f] as f32 * mantissas[f] as f32 * inv2;
        }
        return e1 / e2;
    }

    let mut candidates = Vec::with_capacity(last - first);
    for (j, f) in (first..last).enumerate() {
        let t = input[j] * mul;
        e1 += input[j] * input[j];
        mantissas[f] = to_int(t);
        e2 += mantissas[f] as f32 * mantissas[f] as f32 * inv2;

        let delta = t - (t.trunc() + 0.5);
        if delta.abs() < 0.25 {
            candidates.push((delta, f));
        }
    }

    if candidates.is_empty() {
        return e1 / e2;
    }

    candidates.sort_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));

    if e2 < e1 {
        for (_, f) in candidates {
            let j = f - first;
            let t = input[j] * mul;
            if (mantissas[f].abs() as f32) < t.abs() && (mantissas[f].abs() as f32) < mul - 1.0 {
                let mut m = mantissas[f];
                if m > 0 {
                    m += 1;
                }
                if m < 0 {
                    m -= 1;
                }
                if m == 0 {
                    m = if t > 0.0 { 1 } else { -1 };
                }

                let mut ex = e2;
                ex -= mantissas[f] as f32 * mantissas[f] as f32 * inv2;
                ex += m as f32 * m as f32 * inv2;
                if (ex - e1).abs() < (e2 - e1).abs() {
                    mantissas[f] = m;
                    e2 = ex;
                }
            }
        }
        return e1 / e2;
    }

    if e2 > e1 {
        for (_, f) in candidates {
            let j = f - first;
            let t = input[j] * mul;
            if (mantissas[f].abs() as f32) > t.abs() {
                let mut m = mantissas[f];
                if m > 0 {
                    m -= 1;
                }
                if m < 0 {
                    m += 1;
                }

                let mut ex = e2;
                ex -= mantissas[f] as f32 * mantissas[f] as f32 * inv2;
                ex += m as f32 * m as f32 * inv2;
                if (ex - e1).abs() < (e2 - e1).abs() {
                    mantissas[f] = m;
                    e2 = ex;
                }
            }
        }
    }

    e1 / e2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quant_save_energy_lost() {
        let test_data = [
            (
                [-2.35, -0.84, 0.65, -1.39, 1.25, -0.41, -0.85, 0.89],
                2.35001,
                2.5,
                0.5,
            ),
            (
                [-1.26, 1.26, -1.26, 1.26, -1.26, 1.26, -1.26, 1.26],
                2.35001,
                2.5,
                0.4,
            ),
            (
                [-0.32, 0.13, 0.28, 0.35, 0.63, 0.86, 0.63, 0.04],
                1.0,
                15.5,
                0.03,
            ),
        ];

        for (input, scale, q, diff) in test_data {
            let e1 = input.iter().map(|x| x * x).sum::<f32>();
            let scaled = input.iter().map(|x| x / scale).collect::<Vec<_>>();
            let mut mantissas = vec![0; input.len()];
            quant_mantissas(&scaled, 0, mantissas.len() as u32, q, true, &mut mantissas);

            let e2 = mantissas
                .iter()
                .map(|x| {
                    let t = *x as f32 * (scale / q);
                    t * t
                })
                .sum::<f32>();
            assert!((e2 - e1).abs() < diff, "e1 {e1}, e2 {e2}");
        }
    }

    #[test]
    fn scaler_uses_lower_bound_scale_index() {
        let scaler = Scaler::new(&[0.25, 0.5, 1.0]);
        assert_eq!(0, scaler.scale(&[0.25]).scale_factor_index);
        assert_eq!(1, scaler.scale(&[0.25001]).scale_factor_index);
        assert_eq!(2, scaler.scale(&[2.0]).scale_factor_index);
    }

    #[test]
    fn scale_frame_uses_short_or_long_offsets_per_band() {
        let scaler = Scaler::new(&[0.25, 0.5, 1.0]);
        let specs = (0..16).map(|i| i as f32 / 16.0).collect::<Vec<_>>();
        let layout = BlockLayout {
            num_qmf: 2,
            blocks_per_band: &[0, 1, 2],
            specs_start_short: &[2, 8],
            specs_start_long: &[0, 4],
            specs_per_block: &[2, 2],
            short_windows: &[false, true],
        };

        let blocks = scaler.scale_frame(&specs, &layout);
        assert_eq!(2, blocks.len());
        assert_eq!(specs[0] / 0.25, blocks[0].values[0]);
        assert_eq!(specs[8] / 1.0, blocks[1].values[0]);
    }
}
