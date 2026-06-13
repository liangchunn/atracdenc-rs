use crate::{
    at1::data::{
        BFU_AMOUNT_TAB, BITS_PER_BFU_AMOUNT_TAB_IDX, BITS_PER_IDSF, BITS_PER_IDWL, BLOCKS_PER_BAND,
        BlockSizeMod, MAX_BFUS, NUM_QMF, SOUND_UNIT_SIZE, SPECS_PER_BLOCK,
    },
    atrac::{
        psy::{analyze_scale_factor_spread, calc_ath},
        scale::ScaledBlock,
    },
    bitstream::{BitStream, make_sign},
    util::to_int,
};

const FIXED_BIT_ALLOC_TABLE_LONG: [f32; MAX_BFUS] = [
    7.0, 7.0, 7.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0,
    6.0, 6.0, 6.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 4.0, 4.0, 4.0,
    3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 2.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0,
];

const FIXED_BIT_ALLOC_TABLE_SHORT: [f32; MAX_BFUS] = [
    6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0, 6.0,
    6.0, 6.0, 6.0, 6.0, 6.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 4.0, 4.0,
    4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];

const BIT_BOOST_MASK: [u32; MAX_BFUS] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

#[derive(Debug, Clone)]
pub struct BitsBooster {
    bits_boost_map: Vec<(u32, u32)>,
    max_bits_per_iteration: u32,
    min_key: u32,
}

impl BitsBooster {
    pub fn new() -> Self {
        let mut bits_boost_map = Vec::new();
        for i in 0..MAX_BFUS {
            if BIT_BOOST_MASK[i] != 0 {
                bits_boost_map.push((SPECS_PER_BLOCK[i], i as u32));
            }
        }
        bits_boost_map.sort_unstable();
        let max_bits_per_iteration = bits_boost_map.last().map(|x| x.0).unwrap_or(0);
        let min_key = bits_boost_map.first().map(|x| x.0).unwrap_or(0);
        Self {
            bits_boost_map,
            max_bits_per_iteration,
            min_key,
        }
    }

    pub fn apply_boost(&self, bits_per_each_block: &mut [u32], cur: u32, target: u32) -> u32 {
        let mut surplus = target - cur;
        let key = surplus.min(self.max_bits_per_iteration);
        let max_pos = self
            .bits_boost_map
            .partition_point(|(bits, _)| *bits <= key);
        if max_pos == 0 {
            return surplus;
        }

        while surplus >= self.min_key {
            let mut done = true;
            for (cur_bits, cur_pos) in &self.bits_boost_map[..max_pos] {
                let cur_pos = *cur_pos as usize;
                if cur_pos >= bits_per_each_block.len() {
                    break;
                }
                if bits_per_each_block[cur_pos] == 16 {
                    continue;
                }
                let n_bits_per_spec = if bits_per_each_block[cur_pos] != 0 {
                    1
                } else {
                    2
                };
                if bits_per_each_block[cur_pos] == 0 && cur_bits * 2 > surplus {
                    continue;
                }
                if cur_bits * n_bits_per_spec > surplus {
                    continue;
                }
                bits_per_each_block[cur_pos] += n_bits_per_spec;
                surplus -= cur_bits * n_bits_per_spec;
                done = false;
            }
            if done {
                break;
            }
        }

        surplus
    }
}

impl Default for BitsBooster {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct Atrac1BitAllocator {
    booster: BitsBooster,
    bfu_idx_const: u32,
    ath_long: Vec<f32>,
}

impl Atrac1BitAllocator {
    pub fn new(bfu_idx_const: u32) -> Self {
        debug_assert!(
            bfu_idx_const <= crate::at1::data::MAX_BFU_IDX_CONST,
            "bfu_idx_const out of range; construct via Atrac1Encoder::try_new"
        );
        Self {
            booster: BitsBooster::new(),
            bfu_idx_const,
            ath_long: calc_at1_ath(),
        }
    }

    pub fn encode_frame(
        &self,
        scaled_blocks: &[ScaledBlock],
        block_size: &BlockSizeMod,
        loudness: f32,
    ) -> Vec<u8> {
        let mut bfu_idx = if self.bfu_idx_const != 0 {
            self.bfu_idx_const - 1
        } else {
            7
        };
        let spread = analyze_scale_factor_spread(scaled_blocks);

        let bits_per_each_block = loop {
            let bfu_num = BFU_AMOUNT_TAB[bfu_idx as usize] as usize;
            let target = calc_available_bits_for_bfus(bfu_num);
            let mut min_lambda = -3.0_f32;
            let mut max_lambda = 15.0_f32;
            let mut last_lambda = max_lambda;
            let mut tmp_alloc;
            let mut used_bits;

            loop {
                let shift = if max_lambda <= min_lambda {
                    last_lambda
                } else {
                    (max_lambda + min_lambda) / 2.0
                };
                tmp_alloc = calc_bits_allocation(
                    scaled_blocks,
                    bfu_num,
                    spread,
                    shift,
                    block_size,
                    loudness,
                    &self.ath_long,
                );
                used_bits = calc_bits_used(&tmp_alloc);

                if max_lambda <= min_lambda {
                    break;
                }
                if used_bits < target {
                    last_lambda = shift;
                    max_lambda = shift - 0.01;
                } else if used_bits > target {
                    min_lambda = shift + 0.01;
                } else {
                    break;
                }
            }

            if self.bfu_idx_const == 0 {
                let used_bfu_id = get_max_used_bfu_id(&tmp_alloc);
                if used_bfu_id < bfu_idx {
                    bfu_idx -= 1;
                    continue;
                }
            }

            let mut bits = tmp_alloc;
            let boost_target = calc_available_bits_for_bfus(bits.len());
            self.booster.apply_boost(&mut bits, used_bits, boost_target);
            break bits;
        };

        dump_frame(scaled_blocks, block_size, bfu_idx, &bits_per_each_block)
    }
}

fn calc_at1_ath() -> Vec<f32> {
    let ath_spec = calc_ath(512, 44_100);
    let mut out = Vec::with_capacity(MAX_BFUS);
    for band_num in 0..NUM_QMF {
        let s = BLOCKS_PER_BAND[band_num] as usize;
        let e = BLOCKS_PER_BAND[band_num + 1] as usize;
        for (&specs, &start) in SPECS_PER_BLOCK[s..e]
            .iter()
            .zip(&crate::at1::data::SPECS_START_LONG[s..e])
        {
            let spec_num_start = start as usize;
            let mut x = 999.0_f32;
            for &ath in &ath_spec[spec_num_start..spec_num_start + specs as usize] {
                x = x.min(ath);
            }
            out.push(10.0_f32.powf(0.1 * x));
        }
    }
    out
}

fn calc_bits_allocation(
    scaled_blocks: &[ScaledBlock],
    bfu_num: usize,
    spread: f32,
    shift: f32,
    block_size: &BlockSizeMod,
    loudness: f32,
    ath_long: &[f32],
) -> Vec<u32> {
    let mut bits_per_each_block = vec![0; bfu_num];
    for i in 0..bits_per_each_block.len() {
        let short_block =
            block_size.log_count[crate::at1::data::bfu_to_band(i as u32) as usize] != 0;
        let fix = if short_block {
            FIXED_BIT_ALLOC_TABLE_SHORT[i]
        } else {
            FIXED_BIT_ALLOC_TABLE_LONG[i]
        };
        let ath = ath_long[i] * loudness;
        if !short_block && scaled_blocks[i].energy < ath {
            bits_per_each_block[i] = 0;
        } else {
            let tmp = (spread * (f32::from(scaled_blocks[i].scale_factor_index) / 3.2)
                + (1.0 - spread) * fix
                - shift) as i32;
            bits_per_each_block[i] = if tmp > 16 {
                16
            } else if tmp < 2 {
                0
            } else {
                tmp as u32
            };
        }
    }
    bits_per_each_block
}

fn calc_bits_used(bits_per_each_block: &[u32]) -> u32 {
    bits_per_each_block
        .iter()
        .enumerate()
        .map(|(i, bits)| SPECS_PER_BLOCK[i] * bits)
        .sum()
}

fn get_max_used_bfu_id(bits_per_each_block: &[u32]) -> u32 {
    let mut idx = 7_usize;
    loop {
        let mut bfu_num = BFU_AMOUNT_TAB[idx] as usize;
        if bfu_num > bits_per_each_block.len() {
            idx -= 1;
        } else if idx != 0 {
            let mut i = 0;
            while idx != 0 && bits_per_each_block[bfu_num - 1 - i] == 0 {
                i += 1;
                if i >= (BFU_AMOUNT_TAB[idx] - BFU_AMOUNT_TAB[idx - 1]) as usize {
                    idx -= 1;
                    bfu_num -= i;
                    i = 0;
                }
            }
            break;
        } else {
            break;
        }
    }
    idx as u32
}

pub fn calc_available_bits_for_bfus(bfu_num: usize) -> u32 {
    SOUND_UNIT_SIZE * 8
        - BITS_PER_BFU_AMOUNT_TAB_IDX
        - 32
        - 2
        - 3
        - bfu_num as u32 * (BITS_PER_IDWL + BITS_PER_IDSF)
}

fn dump_frame(
    scaled_blocks: &[ScaledBlock],
    block_size: &BlockSizeMod,
    bfu_idx: u32,
    bits_per_each_block: &[u32],
) -> Vec<u8> {
    let mut bs = BitStream::new();
    bs.write(0x2 - block_size.log_count[0], 2);
    bs.write(0x2 - block_size.log_count[1], 2);
    bs.write(0x3 - block_size.log_count[2], 2);
    bs.write(0, 2);
    bs.write(bfu_idx, BITS_PER_BFU_AMOUNT_TAB_IDX as usize);
    bs.write(0, 2);
    bs.write(0, 3);

    for word_length in bits_per_each_block {
        let tmp = if *word_length != 0 {
            word_length - 1
        } else {
            0
        };
        bs.write(tmp, 4);
    }

    for block in scaled_blocks.iter().take(bits_per_each_block.len()) {
        bs.write(u32::from(block.scale_factor_index), 6);
    }

    for (i, word_length) in bits_per_each_block.iter().enumerate() {
        if *word_length == 0 || *word_length == 1 {
            continue;
        }

        let multiple = ((1_i32 << (word_length - 1)) - 1) as f32;
        for val in &scaled_blocks[i].values {
            let tmp = to_int(val * multiple);
            bs.write(make_sign(tmp, *word_length) as u32, *word_length as usize);
        }
    }

    bs.write(0, 8);
    bs.write(0, 8);
    bs.write(0, 8);
    bs.bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_blocks() -> Vec<ScaledBlock> {
        SPECS_PER_BLOCK
            .iter()
            .enumerate()
            .map(|(idx, len)| ScaledBlock {
                scale_factor_index: (20 + idx % 20) as u8,
                values: (0..*len)
                    .map(|i| ((i as f32 + 1.0) / (*len as f32 + 1.0)).min(0.999))
                    .collect(),
                energy: 1000.0,
            })
            .collect()
    }

    #[test]
    fn available_bits_matches_frame_budget() {
        assert_eq!(1136, calc_available_bits_for_bfus(52));
        assert_eq!(1456, calc_available_bits_for_bfus(20));
    }

    #[test]
    fn encode_frame_produces_decodable_header_and_bounded_size() {
        let alloc = Atrac1BitAllocator::new(8);
        let block_size = BlockSizeMod::default();
        let frame = alloc.encode_frame(&test_blocks(), &block_size, 1.0);
        assert!(frame.len() <= SOUND_UNIT_SIZE as usize);

        let mut bs = BitStream::from_bytes(&frame);
        let parsed = BlockSizeMod::parse(&mut bs);
        assert_eq!(block_size, parsed);
        assert_eq!(7, bs.read(3));
    }

    #[test]
    fn encoded_frame_can_be_dequantised() {
        let alloc = Atrac1BitAllocator::new(8);
        let block_size = BlockSizeMod::new(false, true, false);
        let frame = alloc.encode_frame(&test_blocks(), &block_size, 1.0);

        let mut bs = BitStream::from_bytes(&frame);
        let parsed = BlockSizeMod::parse(&mut bs);
        assert_eq!(block_size, parsed);

        let mut specs = vec![0.0; 512];
        crate::at1::dequantiser::Atrac1Dequantiser::new().dequant(&mut bs, &parsed, &mut specs);
        assert!(specs.iter().all(|x| x.is_finite()));
        assert!(specs.iter().any(|x| x.abs() > 0.0));
    }

    #[test]
    fn bits_booster_spends_surplus_without_exceeding_limits() {
        let booster = BitsBooster::new();
        let mut bits = vec![0; MAX_BFUS];
        let surplus = booster.apply_boost(&mut bits, 0, 100);
        assert!(surplus < 100);
        assert!(bits.iter().all(|x| *x <= 16));
    }
}
