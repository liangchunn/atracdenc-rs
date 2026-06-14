//! ATRAC3+ static tables: inverse-mantissa, scale table, BFU layout, and the
//! Huffman/VLC encode tables built from the FFmpeg-derived count/xlat arrays.
//!
//! Port of `atracdenc/src/atrac/at3p/at3p_tables.{h,cpp}` (LGPL-2.1).

use std::sync::LazyLock;

use super::ff_tables::*;

/// VLC element: code value and bit length.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VlcElement {
    pub code: i16,
    pub len: i16,
}

/// Number of QMF subbands (PQF bands) for ATRAC3+.
pub const NUM_QMF: usize = 16;
/// Maximum number of block-floating units.
pub const MAX_BFUS: usize = 32;

/// `BlocksPerBand[NumQMF + 1]`.
pub const BLOCKS_PER_BAND: [u32; NUM_QMF + 1] = [
    0, 8, 12, 16, 18, 20, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
];

/// `SpecsPerBlock[MaxBfus]`.
pub const SPECS_PER_BLOCK: [u32; MAX_BFUS] = [
    16, 16, 16, 16, 16, 16, 16, 16, 32, 32, 32, 32, 32, 32, 32, 32, 64, 64, 64, 64, 64, 64, 128,
    128, 128, 128, 128, 128, 128, 128, 128, 128,
];

/// `BlockSizeTab[MaxBfus + 1]` (cumulative spec offsets).
pub const BLOCK_SIZE_TAB: [u32; MAX_BFUS + 1] = [
    0, 16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 288, 320, 352, 384, 448, 512, 576,
    640, 704, 768, 896, 1024, 1152, 1280, 1408, 1536, 1664, 1792, 1920, 2048,
];

/// `SpecsStartShort`/`SpecsStartLong` both alias `&BlockSizeTab[0]`.
pub const SPECS_START_SHORT: &[u32] = &BLOCK_SIZE_TAB;
pub const SPECS_START_LONG: &[u32] = &BLOCK_SIZE_TAB;

/// Inverse mantissa table: `Data[0] = 0`, `Data[i] = 1 / mant_tab[i]`.
static INV_MANT_TAB: LazyLock<[f32; 8]> = LazyLock::new(|| {
    let mut d = [0.0f32; 8];
    for i in 1..8 {
        d[i] = 1.0 / ATRAC3P_MANT_TAB[i];
    }
    d
});

pub fn inv_mant_tab(i: usize) -> f32 {
    INV_MANT_TAB[i]
}

/// Scale table (64 entries), normalized so the last entry == 1.0.
pub static SCALE_TABLE: LazyLock<[f32; 64]> = LazyLock::new(|| {
    const SRC: [f32; 64] = [
        0.027852058,
        0.0350914,
        0.044212341,
        0.055704117,
        0.0701828,
        0.088424683,
        0.11140823,
        0.1403656,
        0.17684937,
        0.22281647,
        0.2807312,
        0.35369873,
        0.44563293,
        0.5614624,
        0.70739746,
        0.89126587,
        1.1229248,
        1.4147949,
        1.7825317,
        2.2458496,
        2.8295898,
        3.5650635,
        4.4916992,
        5.6591797,
        7.130127,
        8.9833984,
        11.318359,
        14.260254,
        17.966797,
        22.636719,
        28.520508,
        35.933594,
        45.273438,
        57.041016,
        71.867188,
        90.546875,
        114.08203,
        143.73438,
        181.09375,
        228.16406,
        287.46875,
        362.1875,
        456.32812,
        574.9375,
        724.375,
        912.65625,
        1149.875,
        1448.75,
        1825.3125,
        2299.75,
        2897.5,
        3650.625,
        4599.5,
        5795.0,
        7301.25,
        9199.0,
        11590.0,
        14602.5,
        18398.0,
        23180.0,
        29205.0,
        36796.0,
        46360.0,
        58410.0,
    ];
    let last = SRC[63];
    let mut out = [0.0f32; 64];
    for i in 0..64 {
        out[i] = SRC[i] / last;
    }
    out
});

/// Canonical Huffman encode-table builder (port of `GenHuffmanEncTable`).
///
/// Reads bit-length counts from `cb`, translates each successive symbol index
/// through `xlat`, and assigns sequential codes. Returns the number of symbols
/// consumed (so the caller can advance its `xlat` cursor). Panics if a symbol
/// is out of range (mirrors the C `throw`).
fn gen_huffman_enc_table(cb: &[u8], xlat: &[u8], out: &mut [VlcElement]) -> usize {
    let mut index: usize = 0;
    let mut code: i32 = 0;
    let out_len = out.len();
    for b in 1..=12i16 {
        let mut count = cb[(b - 1) as usize];
        while count > 0 {
            let val = xlat[index] as usize;
            assert!(
                val < out_len,
                "encoded value out of range: {val} >= {out_len}"
            );
            out[val].code = code as i16;
            out[val].len = b;
            index += 1;
            code += 1;
            count -= 1;
        }
        code <<= 1;
    }
    index
}

/// ATRAC3+ Huffman/VLC encode tables (port of `THuffTables`).
pub struct HuffTables {
    pub num_tone_bands: [VlcElement; 16],
    pub vlc_specs: Vec<[VlcElement; 256]>, // 112 entries
    pub word_lens: [[VlcElement; 8]; 4],
    pub code_tables: [[VlcElement; 8]; 4],
}

impl HuffTables {
    pub fn new() -> Self {
        let mut num_tone_bands = [VlcElement::default(); 16];
        gen_huffman_enc_table(
            &ATRAC3P_TONE_CBS[0],
            &ATRAC3P_TONE_XLATS,
            &mut num_tone_bands,
        );

        let mut word_lens = [[VlcElement::default(); 8]; 4];
        let mut code_tables = [[VlcElement::default(); 8]; 4];
        let mut x = 0usize;
        for i in 0..4 {
            x += gen_huffman_enc_table(
                &ATRAC3P_WL_CBS[i],
                &ATRAC3P_WL_CT_XLATS[x..],
                &mut word_lens[i],
            );
            x += gen_huffman_enc_table(
                &ATRAC3P_CT_CBS[i],
                &ATRAC3P_WL_CT_XLATS[x..],
                &mut code_tables[i],
            );
        }

        let mut vlc_specs: Vec<[VlcElement; 256]> = vec![[VlcElement::default(); 256]; 112];
        let mut xx = 0usize;
        for i in 0..112 {
            if ATRAC3P_SPECTRA_CBS[i][0] >= 0 {
                // Build table i in place; need a temporary to avoid borrow issues.
                let mut tmp = [VlcElement::default(); 256];
                let cb: [u8; 12] = std::array::from_fn(|k| ATRAC3P_SPECTRA_CBS[i][k] as u8);
                xx += gen_huffman_enc_table(&cb, &ATRAC3P_SPECTRA_XLATS[xx..], &mut tmp);
                vlc_specs[i] = tmp;
            } else {
                let alias = (-ATRAC3P_SPECTRA_CBS[i][0]) as usize;
                vlc_specs[i] = vlc_specs[alias];
            }
        }

        Self {
            num_tone_bands,
            vlc_specs,
            word_lens,
            code_tables,
        }
    }
}

impl Default for HuffTables {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_table_last_is_one() {
        assert!((SCALE_TABLE[63] - 1.0).abs() < 1e-6);
        assert!(SCALE_TABLE[0] > 0.0 && SCALE_TABLE[0] < SCALE_TABLE[1]);
    }

    #[test]
    fn inv_mant_tab_zero_and_inverse() {
        assert_eq!(inv_mant_tab(0), 0.0);
        assert!((inv_mant_tab(1) - 1.0 / ATRAC3P_MANT_TAB[1]).abs() < 1e-6);
    }

    #[test]
    fn huff_tables_build() {
        // Building must not panic (validates all count/xlat ranges) and the
        // alias rows must be copied from a previously-built table.
        let t = HuffTables::new();
        assert_eq!(t.vlc_specs.len(), 112);
        // num_tone_bands has 16 valid codes with nonzero length.
        for e in &t.num_tone_bands {
            assert!(e.len > 0);
        }
    }
}
