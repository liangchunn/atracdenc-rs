use std::{f32::consts::PI, sync::LazyLock};

use crate::bitstream::BitStream;

pub const MAX_BFUS: usize = 52;
pub const NUM_QMF: usize = 3;
pub const SOUND_UNIT_SIZE: u32 = 212;
pub const NUM_SAMPLES: usize = 512;
pub const BITS_PER_BFU_AMOUNT_TAB_IDX: u32 = 3;
pub const BITS_PER_IDWL: u32 = 4;
pub const BITS_PER_IDSF: u32 = 6;

pub const SPECS_PER_BLOCK: [u32; MAX_BFUS] = [
    8, 8, 8, 8, 4, 4, 4, 4, 8, 8, 8, 8, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 9, 9, 9, 9,
    10, 10, 10, 10, 12, 12, 12, 12, 12, 12, 12, 12, 20, 20, 20, 20, 20, 20, 20, 20,
];
pub const BLOCKS_PER_BAND: [u32; NUM_QMF + 1] = [0, 20, 36, 52];
pub const SPECS_START_LONG: [u32; MAX_BFUS] = [
    0, 8, 16, 24, 32, 36, 40, 44, 48, 56, 64, 72, 80, 86, 92, 98, 104, 110, 116, 122, 128, 134,
    140, 146, 152, 159, 166, 173, 180, 189, 198, 207, 216, 226, 236, 246, 256, 268, 280, 292, 304,
    316, 328, 340, 352, 372, 392, 412, 432, 452, 472, 492,
];
pub const SPECS_START_SHORT: [u32; MAX_BFUS] = [
    0, 32, 64, 96, 8, 40, 72, 104, 12, 44, 76, 108, 20, 52, 84, 116, 26, 58, 90, 122, 128, 160,
    192, 224, 134, 166, 198, 230, 141, 173, 205, 237, 150, 182, 214, 246, 256, 288, 320, 352, 384,
    416, 448, 480, 268, 300, 332, 364, 396, 428, 460, 492,
];
pub const BFU_AMOUNT_TAB: [u32; 8] = [20, 28, 32, 36, 40, 44, 48, 52];

pub static SCALE_TABLE: LazyLock<[f32; 64]> = LazyLock::new(|| {
    let mut table = [0.0; 64];
    for (i, scale) in table.iter_mut().enumerate() {
        *scale = 2.0_f32.powf(i as f32 / 3.0 - 21.0);
    }
    table
});

pub static SINE_WINDOW: LazyLock<[f32; 32]> = LazyLock::new(|| {
    let mut table = [0.0; 32];
    for (i, x) in table.iter_mut().enumerate() {
        *x = ((i as f32 + 0.5) * (PI / 64.0)).sin();
    }
    table
});

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum WindowMode {
    NoTransient,
    Auto,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct EncodeSettings {
    pub bfu_idx_const: u32,
    pub window_mode: WindowMode,
    pub window_mask: u32,
}

impl Default for EncodeSettings {
    fn default() -> Self {
        Self {
            bfu_idx_const: 0,
            window_mode: WindowMode::Auto,
            window_mask: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct BlockSizeMod {
    pub log_count: [u32; NUM_QMF],
}

impl BlockSizeMod {
    pub fn new(low_short: bool, mid_short: bool, hi_short: bool) -> Self {
        Self {
            log_count: [
                if low_short { 2 } else { 0 },
                if mid_short { 2 } else { 0 },
                if hi_short { 3 } else { 0 },
            ],
        }
    }

    pub fn parse(stream: &mut BitStream) -> Self {
        let low = 2 - stream.read(2);
        let mid = 2 - stream.read(2);
        let hi = 3 - stream.read(2);
        stream.read(2);
        Self {
            log_count: [low, mid, hi],
        }
    }

    pub fn short_win(&self, band: usize) -> bool {
        self.log_count[band] != 0
    }
}

pub fn bfu_to_band(i: u32) -> u32 {
    if i < 20 {
        0
    } else if i < 36 {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_match_expected_shapes() {
        assert_eq!(MAX_BFUS, SPECS_PER_BLOCK.len());
        assert_eq!(
            512,
            SPECS_START_LONG[MAX_BFUS - 1] + SPECS_PER_BLOCK[MAX_BFUS - 1]
        );
        assert_eq!(
            512,
            SPECS_START_SHORT[MAX_BFUS - 1] + SPECS_PER_BLOCK[MAX_BFUS - 1]
        );
        assert!((SCALE_TABLE[0] - 2.0_f32.powf(-21.0)).abs() < 1.0e-12);
        assert!((SCALE_TABLE[63] - 1.0).abs() < 1.0e-6);
        assert!((SINE_WINDOW[0] - (PI / 64.0 * 0.5).sin()).abs() < 1.0e-7);
    }

    #[test]
    fn block_size_parse_matches_cpp_encoding() {
        let mut bs = BitStream::new();
        bs.write(0, 2);
        bs.write(2, 2);
        bs.write(1, 2);
        bs.write(0, 2);
        let parsed = BlockSizeMod::parse(&mut bs);
        assert_eq!([2, 0, 2], parsed.log_count);
    }
}
