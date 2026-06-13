use crate::{
    at1::data::BlockSizeMod,
    at1::data::{
        BFU_AMOUNT_TAB, BLOCKS_PER_BAND, MAX_BFUS, NUM_QMF, SCALE_TABLE, SPECS_PER_BLOCK,
        SPECS_START_LONG, SPECS_START_SHORT,
    },
    bitstream::{BitStream, make_sign},
};

pub struct Atrac1Dequantiser;

impl Atrac1Dequantiser {
    pub fn new() -> Self {
        Self
    }

    pub fn dequant(&mut self, stream: &mut BitStream, bs: &BlockSizeMod, specs: &mut [f32]) {
        assert!(specs.len() >= 512);
        let mut word_lens = [0_u32; MAX_BFUS];
        let mut id_scale_factors = [0_u32; MAX_BFUS];
        let num_bfus = BFU_AMOUNT_TAB[stream.read(3) as usize] as usize;
        stream.read(2);
        stream.read(3);

        for wl in word_lens.iter_mut().take(num_bfus) {
            *wl = stream.read(4);
        }
        for sf in id_scale_factors.iter_mut().take(num_bfus) {
            *sf = stream.read(6);
        }

        for band_num in 0..NUM_QMF {
            for bfu_num in
                BLOCKS_PER_BAND[band_num] as usize..BLOCKS_PER_BAND[band_num + 1] as usize
            {
                let num_specs = SPECS_PER_BLOCK[bfu_num] as usize;
                let word_len = u32::from(word_lens[bfu_num] != 0) + word_lens[bfu_num];
                let scale_factor = SCALE_TABLE[id_scale_factors[bfu_num] as usize];
                let start_pos = if bs.log_count[band_num] != 0 {
                    SPECS_START_SHORT[bfu_num]
                } else {
                    SPECS_START_LONG[bfu_num]
                } as usize;
                if word_len != 0 {
                    let max_quant = 1.0 / ((1_i32 << (word_len - 1)) - 1) as f32;
                    let scale = scale_factor * max_quant;
                    for i in 0..num_specs {
                        specs[start_pos + i] = scale
                            * make_sign(stream.read(word_len as usize) as i32, word_len) as f32;
                    }
                } else {
                    specs[start_pos..start_pos + num_specs].fill(0.0);
                }
            }
        }
    }
}

impl Default for Atrac1Dequantiser {
    fn default() -> Self {
        Self::new()
    }
}
