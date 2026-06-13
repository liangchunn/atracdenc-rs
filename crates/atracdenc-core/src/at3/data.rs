use std::sync::LazyLock;

use crate::dsp::gain::{GainParams, GainPoint};

pub const MAX_BFUS: usize = 32;
pub const NUM_QMF: usize = 4;
pub const NUM_SAMPLES: usize = 1024;
pub const NUM_SPECS: usize = NUM_SAMPLES;
pub const MDCT_SZ: usize = 512;
pub const FRAME_SZ: u32 = 152;
pub const MAX_SPECS: usize = NUM_SAMPLES;
pub const MAX_SPECS_PER_BLOCK: usize = 128;
pub const EXPONENT_OFFSET: i32 = 4;
pub const LOC_SCALE: u32 = 3;
pub const LOC_SZ: u32 = 1 << LOC_SCALE;
pub const GAIN_INTERPOLATION_POS_SHIFT: i32 = 15;

pub const MAX_QUANT: [f32; 8] = [0.0, 1.5, 2.5, 3.5, 4.5, 7.5, 15.5, 31.5];
pub const BLOCK_SIZE_TAB: [u32; 33] = [
    0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 288, 320,
    352, 384, 416, 448, 480, 512, 576, 640, 704, 768, 896, 1024,
];
pub const SPECS_START_SHORT: [u32; 33] = BLOCK_SIZE_TAB;
pub const SPECS_START_LONG: [u32; 33] = BLOCK_SIZE_TAB;
pub const CLC_LENGTH_TAB: [u32; 8] = [0, 4, 3, 3, 4, 4, 5, 6];
pub const BLOCKS_PER_BAND: [u32; NUM_QMF + 1] = [0, 18, 26, 30, 32];
pub const SPECS_PER_BLOCK: [u32; 33] = [
    8, 8, 8, 8, 8, 8, 8, 8, 16, 16, 16, 16, 16, 16, 16, 16, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32,
    64, 64, 64, 64, 128, 128, 128,
];

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ContainerParams {
    pub bitrate: u32,
    pub frame_sz: u16,
    pub joint_stereo: bool,
}

pub const CONTAINER_PARAMS: [ContainerParams; 8] = [
    ContainerParams {
        bitrate: 66_150,
        frame_sz: 192,
        joint_stereo: true,
    },
    ContainerParams {
        bitrate: 93_713,
        frame_sz: 272,
        joint_stereo: true,
    },
    ContainerParams {
        bitrate: 104_738,
        frame_sz: 304,
        joint_stereo: false,
    },
    ContainerParams {
        bitrate: 132_300,
        frame_sz: 384,
        joint_stereo: false,
    },
    ContainerParams {
        bitrate: 146_081,
        frame_sz: 424,
        joint_stereo: false,
    },
    ContainerParams {
        bitrate: 176_400,
        frame_sz: 512,
        joint_stereo: false,
    },
    ContainerParams {
        bitrate: 264_600,
        frame_sz: 768,
        joint_stereo: false,
    },
    ContainerParams {
        bitrate: 352_800,
        frame_sz: 1024,
        joint_stereo: false,
    },
];

pub const LP4: ContainerParams = CONTAINER_PARAMS[0];
pub const LP2: ContainerParams = CONTAINER_PARAMS[3];

pub fn params_for_bitrate(bitrate: u32) -> Option<ContainerParams> {
    let bitrate = if bitrate == 0 { LP2.bitrate } else { bitrate };
    CONTAINER_PARAMS
        .iter()
        .copied()
        .find(|params| params.bitrate >= bitrate)
}

pub static SCALE_TABLE: LazyLock<[f32; 64]> = LazyLock::new(|| {
    let mut table = [0.0; 64];
    for (i, scale) in table.iter_mut().enumerate() {
        *scale = 2.0_f32.powf(i as f32 / 3.0 - 21.0);
    }
    table
});

pub static ENCODE_WINDOW: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut table = [0.0; 256];
    for (i, window) in table.iter_mut().enumerate() {
        *window = (((((i as f64 + 0.5) / 256.0) - 0.5) * std::f64::consts::PI).sin() + 1.0) as f32;
    }
    table
});

pub static DECODE_WINDOW: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut table = [0.0; 256];
    for i in 0..256 {
        let a = ENCODE_WINDOW[i];
        let b = ENCODE_WINDOW[255 - i];
        table[i] = 2.0 * a / (a * a + b * b);
    }
    table
});

pub const GAIN_LEVEL: [f32; 16] = [
    16.0,
    8.0,
    4.0,
    2.0,
    1.0,
    0.5,
    0.25,
    0.125,
    0.0625,
    0.03125,
    0.015625,
    0.0078125,
    0.00390625,
    0.001953125,
    0.0009765625,
    0.00048828125,
];

pub const GAIN_INTERPOLATION: [f32; 31] = [
    3.6680162,
    3.3635857,
    3.0844216,
    2.828427,
    2.5936792,
    2.3784142,
    2.1810155,
    2.0,
    1.8340081,
    1.6817929,
    1.5422108,
    std::f32::consts::SQRT_2,
    1.2968396,
    1.1892071,
    1.0905077,
    1.0,
    0.91700405,
    0.8408964,
    0.7711054,
    0.70710677,
    0.6484198,
    0.59460354,
    0.5452539,
    0.5,
    0.45850202,
    0.4204482,
    0.3855527,
    0.35355338,
    0.3242099,
    0.29730177,
    0.27262694,
];

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct BlockSizeMod;

impl BlockSizeMod {
    pub fn short_win(&self, _band: usize) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct HuffEntry {
    pub code: u8,
    pub bits: u8,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct HuffTablePair {
    pub table: &'static [HuffEntry],
}

pub const HUFF_TABLE_1: [HuffEntry; 9] = [
    HuffEntry { code: 0x0, bits: 1 },
    HuffEntry { code: 0x4, bits: 3 },
    HuffEntry { code: 0x5, bits: 3 },
    HuffEntry { code: 0xC, bits: 4 },
    HuffEntry { code: 0xD, bits: 4 },
    HuffEntry {
        code: 0x1C,
        bits: 5,
    },
    HuffEntry {
        code: 0x1D,
        bits: 5,
    },
    HuffEntry {
        code: 0x1E,
        bits: 5,
    },
    HuffEntry {
        code: 0x1F,
        bits: 5,
    },
];

pub const HUFF_TABLE_2: [HuffEntry; 5] = [
    HuffEntry { code: 0x0, bits: 1 },
    HuffEntry { code: 0x4, bits: 3 },
    HuffEntry { code: 0x5, bits: 3 },
    HuffEntry { code: 0x6, bits: 3 },
    HuffEntry { code: 0x7, bits: 3 },
];

pub const HUFF_TABLE_3: [HuffEntry; 7] = [
    HuffEntry { code: 0x0, bits: 1 },
    HuffEntry { code: 0x4, bits: 3 },
    HuffEntry { code: 0x5, bits: 3 },
    HuffEntry { code: 0xC, bits: 4 },
    HuffEntry { code: 0xD, bits: 4 },
    HuffEntry { code: 0xE, bits: 4 },
    HuffEntry { code: 0xF, bits: 4 },
];

pub const HUFF_TABLE_5: [HuffEntry; 15] = [
    HuffEntry { code: 0x0, bits: 2 },
    HuffEntry { code: 0x2, bits: 3 },
    HuffEntry { code: 0x3, bits: 3 },
    HuffEntry { code: 0x8, bits: 4 },
    HuffEntry { code: 0x9, bits: 4 },
    HuffEntry { code: 0xA, bits: 4 },
    HuffEntry { code: 0xB, bits: 4 },
    HuffEntry {
        code: 0x1C,
        bits: 5,
    },
    HuffEntry {
        code: 0x1D,
        bits: 5,
    },
    HuffEntry {
        code: 0x3C,
        bits: 6,
    },
    HuffEntry {
        code: 0x3D,
        bits: 6,
    },
    HuffEntry {
        code: 0x3E,
        bits: 6,
    },
    HuffEntry {
        code: 0x3F,
        bits: 6,
    },
    HuffEntry { code: 0xC, bits: 4 },
    HuffEntry { code: 0xD, bits: 4 },
];

pub const HUFF_TABLE_6: [HuffEntry; 31] = [
    HuffEntry { code: 0x0, bits: 3 },
    HuffEntry { code: 0x2, bits: 4 },
    HuffEntry { code: 0x3, bits: 4 },
    HuffEntry { code: 0x4, bits: 4 },
    HuffEntry { code: 0x5, bits: 4 },
    HuffEntry { code: 0x6, bits: 4 },
    HuffEntry { code: 0x7, bits: 4 },
    HuffEntry {
        code: 0x14,
        bits: 5,
    },
    HuffEntry {
        code: 0x15,
        bits: 5,
    },
    HuffEntry {
        code: 0x16,
        bits: 5,
    },
    HuffEntry {
        code: 0x17,
        bits: 5,
    },
    HuffEntry {
        code: 0x18,
        bits: 5,
    },
    HuffEntry {
        code: 0x19,
        bits: 5,
    },
    HuffEntry {
        code: 0x34,
        bits: 6,
    },
    HuffEntry {
        code: 0x35,
        bits: 6,
    },
    HuffEntry {
        code: 0x36,
        bits: 6,
    },
    HuffEntry {
        code: 0x37,
        bits: 6,
    },
    HuffEntry {
        code: 0x38,
        bits: 6,
    },
    HuffEntry {
        code: 0x39,
        bits: 6,
    },
    HuffEntry {
        code: 0x3A,
        bits: 6,
    },
    HuffEntry {
        code: 0x3B,
        bits: 6,
    },
    HuffEntry {
        code: 0x78,
        bits: 7,
    },
    HuffEntry {
        code: 0x79,
        bits: 7,
    },
    HuffEntry {
        code: 0x7A,
        bits: 7,
    },
    HuffEntry {
        code: 0x7B,
        bits: 7,
    },
    HuffEntry {
        code: 0x7C,
        bits: 7,
    },
    HuffEntry {
        code: 0x7D,
        bits: 7,
    },
    HuffEntry {
        code: 0x7E,
        bits: 7,
    },
    HuffEntry {
        code: 0x7F,
        bits: 7,
    },
    HuffEntry { code: 0x8, bits: 4 },
    HuffEntry { code: 0x9, bits: 4 },
];

pub const HUFF_TABLE_7: [HuffEntry; 63] = [
    HuffEntry { code: 0x0, bits: 3 },
    HuffEntry { code: 0x8, bits: 5 },
    HuffEntry { code: 0x9, bits: 5 },
    HuffEntry { code: 0xA, bits: 5 },
    HuffEntry { code: 0xB, bits: 5 },
    HuffEntry { code: 0xC, bits: 5 },
    HuffEntry { code: 0xD, bits: 5 },
    HuffEntry { code: 0xE, bits: 5 },
    HuffEntry { code: 0xF, bits: 5 },
    HuffEntry {
        code: 0x10,
        bits: 5,
    },
    HuffEntry {
        code: 0x11,
        bits: 5,
    },
    HuffEntry {
        code: 0x24,
        bits: 6,
    },
    HuffEntry {
        code: 0x25,
        bits: 6,
    },
    HuffEntry {
        code: 0x26,
        bits: 6,
    },
    HuffEntry {
        code: 0x27,
        bits: 6,
    },
    HuffEntry {
        code: 0x28,
        bits: 6,
    },
    HuffEntry {
        code: 0x29,
        bits: 6,
    },
    HuffEntry {
        code: 0x2A,
        bits: 6,
    },
    HuffEntry {
        code: 0x2B,
        bits: 6,
    },
    HuffEntry {
        code: 0x2C,
        bits: 6,
    },
    HuffEntry {
        code: 0x2D,
        bits: 6,
    },
    HuffEntry {
        code: 0x2E,
        bits: 6,
    },
    HuffEntry {
        code: 0x2F,
        bits: 6,
    },
    HuffEntry {
        code: 0x30,
        bits: 6,
    },
    HuffEntry {
        code: 0x31,
        bits: 6,
    },
    HuffEntry {
        code: 0x32,
        bits: 6,
    },
    HuffEntry {
        code: 0x33,
        bits: 6,
    },
    HuffEntry {
        code: 0x68,
        bits: 7,
    },
    HuffEntry {
        code: 0x69,
        bits: 7,
    },
    HuffEntry {
        code: 0x6A,
        bits: 7,
    },
    HuffEntry {
        code: 0x6B,
        bits: 7,
    },
    HuffEntry {
        code: 0x6C,
        bits: 7,
    },
    HuffEntry {
        code: 0x6D,
        bits: 7,
    },
    HuffEntry {
        code: 0x6E,
        bits: 7,
    },
    HuffEntry {
        code: 0x6F,
        bits: 7,
    },
    HuffEntry {
        code: 0x70,
        bits: 7,
    },
    HuffEntry {
        code: 0x71,
        bits: 7,
    },
    HuffEntry {
        code: 0x72,
        bits: 7,
    },
    HuffEntry {
        code: 0x73,
        bits: 7,
    },
    HuffEntry {
        code: 0x74,
        bits: 7,
    },
    HuffEntry {
        code: 0x75,
        bits: 7,
    },
    HuffEntry {
        code: 0xEC,
        bits: 8,
    },
    HuffEntry {
        code: 0xED,
        bits: 8,
    },
    HuffEntry {
        code: 0xEE,
        bits: 8,
    },
    HuffEntry {
        code: 0xEF,
        bits: 8,
    },
    HuffEntry {
        code: 0xF0,
        bits: 8,
    },
    HuffEntry {
        code: 0xF1,
        bits: 8,
    },
    HuffEntry {
        code: 0xF2,
        bits: 8,
    },
    HuffEntry {
        code: 0xF3,
        bits: 8,
    },
    HuffEntry {
        code: 0xF4,
        bits: 8,
    },
    HuffEntry {
        code: 0xF5,
        bits: 8,
    },
    HuffEntry {
        code: 0xF6,
        bits: 8,
    },
    HuffEntry {
        code: 0xF7,
        bits: 8,
    },
    HuffEntry {
        code: 0xF8,
        bits: 8,
    },
    HuffEntry {
        code: 0xF9,
        bits: 8,
    },
    HuffEntry {
        code: 0xFA,
        bits: 8,
    },
    HuffEntry {
        code: 0xFB,
        bits: 8,
    },
    HuffEntry {
        code: 0xFC,
        bits: 8,
    },
    HuffEntry {
        code: 0xFD,
        bits: 8,
    },
    HuffEntry {
        code: 0xFE,
        bits: 8,
    },
    HuffEntry {
        code: 0xFF,
        bits: 8,
    },
    HuffEntry { code: 0x2, bits: 4 },
    HuffEntry { code: 0x3, bits: 4 },
];

pub const HUFF_TABLES: [HuffTablePair; 7] = [
    HuffTablePair {
        table: &HUFF_TABLE_1,
    },
    HuffTablePair {
        table: &HUFF_TABLE_2,
    },
    HuffTablePair {
        table: &HUFF_TABLE_3,
    },
    HuffTablePair {
        table: &HUFF_TABLE_1,
    },
    HuffTablePair {
        table: &HUFF_TABLE_5,
    },
    HuffTablePair {
        table: &HUFF_TABLE_6,
    },
    HuffTablePair {
        table: &HUFF_TABLE_7,
    },
];

pub fn mantissa_to_clc_idx(mantissa: i32) -> u32 {
    assert!(mantissa > -3 && mantissa < 2);
    const MANTISSA_CLC_RTAB: [u32; 4] = [2, 3, 0, 1];
    MANTISSA_CLC_RTAB[(mantissa + 2) as usize]
}

pub fn mantissas_to_vlc_index(a: i32, b: i32) -> u32 {
    assert!(a > -2 && a < 2);
    assert!(b > -2 && b < 2);
    const MANTISSAS_VLC_RTAB: [u32; 9] = [8, 4, 7, 2, 0, 1, 6, 3, 5];
    let idx = 3 * (a + 1) + (b + 1);
    MANTISSAS_VLC_RTAB[idx as usize]
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubbandInfo {
    info: Vec<Vec<GainPoint>>,
}

impl SubbandInfo {
    pub const MAX_GAIN_POINTS_NUM: usize = 8;

    pub fn new() -> Self {
        Self {
            info: vec![Vec::new(); NUM_QMF],
        }
    }

    pub fn add_subband_curve(&mut self, n: usize, curve: Vec<GainPoint>) {
        assert!(n < NUM_QMF);
        assert!(curve.len() <= Self::MAX_GAIN_POINTS_NUM);
        self.info[n] = curve;
    }

    pub fn qmf_num(&self) -> usize {
        self.info.len()
    }

    pub fn gain_points(&self, i: usize) -> &[GainPoint] {
        &self.info[i]
    }

    pub fn reset(&mut self) {
        for points in &mut self.info {
            points.clear();
        }
    }
}

impl Default for SubbandInfo {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TonalVal {
    pub pos: u16,
    pub val: f64,
    pub bfu: u8,
}

pub type TonalComponents = Vec<TonalVal>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainEnergyScale {
    pub prev_half: f32,
    pub cur_half: f32,
    pub frame: f32,
}

impl Default for GainEnergyScale {
    fn default() -> Self {
        Self {
            prev_half: 1.0,
            cur_half: 1.0,
            frame: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BfuAllocMode {
    Fast,
    Parity,
}

impl Default for BfuAllocMode {
    fn default() -> Self {
        Self::Fast
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct EncodeSettings {
    pub container_params: ContainerParams,
    pub no_gain_control: bool,
    pub no_tonal_components: bool,
    pub source_channels: u8,
    pub bfu_idx_const: u32,
    pub bfu_alloc_mode: BfuAllocMode,
}

impl EncodeSettings {
    pub fn new(
        bitrate: u32,
        no_gain_control: bool,
        no_tonal_components: bool,
        source_channels: u8,
        bfu_idx_const: u32,
    ) -> Option<Self> {
        Some(Self {
            container_params: params_for_bitrate(bitrate)?,
            no_gain_control,
            no_tonal_components,
            source_channels,
            bfu_idx_const,
            bfu_alloc_mode: BfuAllocMode::Fast,
        })
    }
}

impl Default for EncodeSettings {
    fn default() -> Self {
        Self {
            container_params: LP2,
            no_gain_control: false,
            no_tonal_components: false,
            source_channels: 2,
            bfu_idx_const: 0,
            bfu_alloc_mode: BfuAllocMode::Fast,
        }
    }
}

pub struct At3GainParams;

impl GainParams for At3GainParams {
    const GAIN_LEVEL: &'static [f32] = &GAIN_LEVEL;
    const GAIN_INTERPOLATION: &'static [f32] = &GAIN_INTERPOLATION;
    const EXPONENT_OFFSET: i32 = EXPONENT_OFFSET;
    const GAIN_INTERPOLATION_POS_SHIFT: i32 = GAIN_INTERPOLATION_POS_SHIFT;
    const LOC_SCALE: u32 = LOC_SCALE;
    const LOC_SZ: u32 = LOC_SZ;
    const MDCT_SZ: usize = MDCT_SZ;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::gain::GainProcessor;

    #[test]
    fn tables_match_expected_shapes() {
        assert_eq!(MAX_BFUS, BLOCKS_PER_BAND[NUM_QMF] as usize);
        assert_eq!(
            1024,
            SPECS_START_LONG[MAX_BFUS - 1] + SPECS_PER_BLOCK[MAX_BFUS - 1]
        );
        assert_eq!(SPECS_START_SHORT, SPECS_START_LONG);
        assert!((SCALE_TABLE[0] - 2.0_f32.powf(-21.0)).abs() < 1.0e-12);
        assert!((SCALE_TABLE[63] - 1.0).abs() < 1.0e-6);
        assert!(ENCODE_WINDOW[0] > 0.0);
        assert!((GAIN_LEVEL[0] - 16.0).abs() < 1.0e-6);
        assert!((GAIN_INTERPOLATION[15] - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn bitrate_lookup_matches_cpp_lower_bound_defaulting() {
        assert_eq!(Some(LP2), params_for_bitrate(0));
        assert_eq!(Some(LP4), params_for_bitrate(66_150));
        assert_eq!(Some(CONTAINER_PARAMS[1]), params_for_bitrate(66_151));
        assert_eq!(Some(LP2), params_for_bitrate(132_300));
        assert_eq!(None, params_for_bitrate(352_801));
    }

    #[test]
    fn subband_info_stores_and_resets_gain_points() {
        let mut info = SubbandInfo::new();
        assert_eq!(NUM_QMF, info.qmf_num());

        let points = vec![GainPoint {
            level: 3,
            location: 4,
        }];
        info.add_subband_curve(2, points.clone());
        assert_eq!(&points, info.gain_points(2));

        info.reset();
        assert!(info.gain_points(2).is_empty());
    }

    #[test]
    fn helper_tables_match_expected_reverse_mappings() {
        assert_eq!(2, mantissa_to_clc_idx(-2));
        assert_eq!(0, mantissa_to_clc_idx(0));
        assert_eq!(5, mantissas_to_vlc_index(1, 1));
        assert_eq!(7, HUFF_TABLES.len());
        assert_eq!(63, HUFF_TABLES[6].table.len());
    }

    #[test]
    fn gain_params_are_usable_by_shared_processor() {
        type Gp = GainProcessor<At3GainParams>;

        let mut cur = vec![2.0; MDCT_SZ / 2];
        let mut next = vec![4.0; MDCT_SZ / 2];
        let gain = [GainPoint {
            level: 2,
            location: 1,
        }];

        Gp::modulate(&gain, &mut cur, &mut next);
        assert!(cur.iter().all(|x| x.is_finite()));
        assert!(next.iter().all(|x| x.is_finite()));
    }
}
