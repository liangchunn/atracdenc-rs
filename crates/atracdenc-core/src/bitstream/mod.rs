pub mod encode;

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct BitStream {
    buf: Vec<u8>,
    bits_used: usize,
    read_pos: usize,
}

impl BitStream {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_bytes(buf: &[u8]) -> Self {
        Self {
            buf: buf.to_vec(),
            bits_used: buf.len() * 8,
            read_pos: 0,
        }
    }

    pub fn write(&mut self, val: u32, n: usize) {
        assert!(n <= 23, "bitstream writes are limited to 23 bits");
        if n == 0 {
            return;
        }

        let needed_bits = self.bits_used + n;
        let needed_bytes = needed_bits.div_ceil(8);
        if self.buf.len() < needed_bytes {
            self.buf.resize(needed_bytes, 0);
        }

        for bit_idx in (0..n).rev() {
            let bit = ((val >> bit_idx) & 1) as u8;
            let pos = self.bits_used;
            let byte_pos = pos / 8;
            let bit_pos = 7 - (pos % 8);
            self.buf[byte_pos] |= bit << bit_pos;
            self.bits_used += 1;
        }
    }

    pub fn read(&mut self, n: usize) -> u32 {
        assert!(n <= 23, "bitstream reads are limited to 23 bits");
        assert!(self.read_pos + n <= self.buf.len() * 8, "read past buffer");

        let mut out = 0_u32;
        for _ in 0..n {
            let byte_pos = self.read_pos / 8;
            let bit_pos = 7 - (self.read_pos % 8);
            out = (out << 1) | u32::from((self.buf[byte_pos] >> bit_pos) & 1);
            self.read_pos += 1;
        }
        out
    }

    pub fn size_in_bits(&self) -> usize {
        self.bits_used
    }

    pub fn buf_size(&self) -> usize {
        self.buf.len()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }
}

pub fn make_sign(val: i32, bits: u32) -> i32 {
    assert!((1..=32).contains(&bits));
    let shift = 32 - bits;
    val.wrapping_shl(shift) >> shift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_constructor() {
        let bs = BitStream::new();
        assert_eq!(0, bs.size_in_bits());
    }

    #[test]
    fn simple_write_read() {
        let mut bs = BitStream::new();
        bs.write(5, 3);
        bs.write(1, 1);
        assert_eq!(4, bs.size_in_bits());
        assert_eq!(5, bs.read(3));
        assert_eq!(1, bs.read(1));
    }

    #[test]
    fn overlap_write_read() {
        let mut bs = BitStream::new();
        bs.write(101, 22);
        assert_eq!(22, bs.size_in_bits());

        bs.write(212, 22);
        assert_eq!(44, bs.size_in_bits());

        bs.write(323, 22);
        assert_eq!(66, bs.size_in_bits());

        assert_eq!(101, bs.read(22));
        assert_eq!(212, bs.read(22));
        assert_eq!(323, bs.read(22));
    }

    #[test]
    fn overlap_write_read_2() {
        let mut bs = BitStream::new();
        bs.write(2, 2);
        bs.write(7, 4);
        bs.write(10003, 16);

        assert_eq!(2, bs.read(2));
        assert_eq!(7, bs.read(4));
        assert_eq!(10003, bs.read(16));
    }

    #[test]
    fn overlap_write_read_3() {
        let mut bs = BitStream::new();
        let values = [
            (40, 6),
            (3, 2),
            (0, 3),
            (0, 3),
            (0, 3),
            (0, 3),
            (3, 5),
            (1, 2),
            (1, 1),
            (1, 1),
            (1, 1),
            (1, 1),
            (0, 3),
            (4, 3),
            (35, 6),
            (25, 6),
            (3, 3),
            (32, 6),
            (29, 6),
            (3, 3),
            (36, 6),
            (49, 6),
        ];

        for (val, bits) in values {
            bs.write(val, bits);
        }

        for (val, bits) in values {
            assert_eq!(val, bs.read(bits));
        }
    }

    #[test]
    fn sign_write_read() {
        let mut bs = BitStream::new();
        bs.write(make_sign(-2, 3) as u32, 3);
        bs.write(make_sign(-1, 3) as u32, 3);
        bs.write(make_sign(1, 2) as u32, 2);
        bs.write(make_sign(-7, 4) as u32, 4);

        assert_eq!(-2, make_sign(bs.read(3) as i32, 3));
        assert_eq!(-1, make_sign(bs.read(3) as i32, 3));
        assert_eq!(1, make_sign(bs.read(2) as i32, 2));
        assert_eq!(-7, make_sign(bs.read(4) as i32, 4));
    }

    #[test]
    fn fixed_seed_round_trip_property() {
        let mut state = 0x1234_5678_u32;
        let mut seq = Vec::new();
        let mut bs = BitStream::new();

        for _ in 0..300 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = (state as usize % 23) + 1;
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let mask = (1_u32 << n) - 1;
            let val = state & mask;
            seq.push((val, n));
            bs.write(val, n);
        }

        for (val, n) in seq {
            assert_eq!(val, bs.read(n));
        }
    }
}
