use super::BitStream;

#[derive(Debug, Clone)]
pub struct BitAllocHandler {
    target_bits: usize,
    min_lambda: f32,
    max_lambda: f32,
    cur_lambda: f32,
    last_lambda: f32,
    need_repeat: bool,
    cur_enc_pos: usize,
    repeat_enc_pos: usize,
    previous_consumption: u32,
}

impl Default for BitAllocHandler {
    fn default() -> Self {
        Self {
            target_bits: 0,
            min_lambda: 0.0,
            max_lambda: 0.0,
            cur_lambda: 0.0,
            last_lambda: 0.0,
            need_repeat: false,
            cur_enc_pos: 0,
            repeat_enc_pos: 0,
            previous_consumption: 0,
        }
    }
}

impl BitAllocHandler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, target_bits: usize, min_lambda: f32, max_lambda: f32) {
        self.target_bits = target_bits;
        self.min_lambda = min_lambda;
        self.max_lambda = max_lambda;
        self.last_lambda = max_lambda;
    }

    pub fn cont(&mut self) -> f32 {
        if self.max_lambda <= self.min_lambda {
            return self.last_lambda;
        }

        self.cur_lambda = (self.max_lambda + self.min_lambda) / 2.0;
        self.repeat_enc_pos = self.cur_enc_pos;
        self.cur_lambda
    }

    pub fn check(&self, got_bits: usize) -> bool {
        got_bits < self.target_bits
    }

    pub fn submit(&mut self, got_bits: usize) -> bool {
        if self.max_lambda <= self.min_lambda {
            self.need_repeat = false;
        } else if got_bits < self.target_bits {
            self.last_lambda = self.cur_lambda;
            self.max_lambda = self.cur_lambda - 0.01;
            self.need_repeat = true;
        } else if got_bits > self.target_bits {
            self.min_lambda = self.cur_lambda + 0.01;
            self.need_repeat = true;
        } else {
            self.need_repeat = false;
        }

        !self.need_repeat
    }

    pub fn cur_global_consumption(&self) -> u32 {
        self.previous_consumption
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum EncodeStatus {
    Ok,
    Repeat,
}

/// Errors produced by a bitstream part encoder.
///
/// This framework is internal and codec-specific (currently only ATRAC3+);
/// the trait is not meant to be implemented by library consumers.
#[derive(Debug, thiserror::Error)]
pub enum BitStreamEncodeError {
    #[error("tonal bands ({tone_bands}) exceed quant units ({quant_units})")]
    TonalBandsExceedQuantUnits { tone_bands: u8, quant_units: u32 },
}

pub trait BitStreamPartEncoder<TFrame> {
    fn encode(
        &mut self,
        frame: &mut TFrame,
        ba: &mut BitAllocHandler,
    ) -> Result<EncodeStatus, BitStreamEncodeError>;
    fn dump(&mut self, bs: &mut BitStream);
    fn reset(&mut self) {}
    fn consumption(&self) -> u32;
}

pub struct BitStreamEncoder<TFrame> {
    parts: Vec<Box<dyn BitStreamPartEncoder<TFrame>>>,
}

impl<TFrame> BitStreamEncoder<TFrame> {
    pub fn new(parts: Vec<Box<dyn BitStreamPartEncoder<TFrame>>>) -> Self {
        Self { parts }
    }

    pub fn run(
        &mut self,
        frame: &mut TFrame,
        bs: &mut BitStream,
    ) -> Result<(), BitStreamEncodeError> {
        let mut ba = BitAllocHandler::new();
        let mut cont;

        loop {
            cont = false;
            let mut cur_enc_pos = ba.repeat_enc_pos;

            while cur_enc_pos < self.parts.len() {
                ba.cur_enc_pos = cur_enc_pos;
                ba.previous_consumption = self.parts[..cur_enc_pos]
                    .iter()
                    .map(|part| part.consumption())
                    .sum();

                let status = self.parts[cur_enc_pos].encode(frame, &mut ba)?;

                if ba.need_repeat {
                    ba.need_repeat = false;
                    cont = true;
                    break;
                }

                if status == EncodeStatus::Repeat {
                    cont = true;
                    for part in &mut self.parts[..cur_enc_pos] {
                        part.reset();
                    }
                    ba.repeat_enc_pos = 0;
                    break;
                }

                cur_enc_pos += 1;
            }

            if !cont {
                break;
            }
        }

        for part in &mut self.parts {
            part.dump(bs);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some_bit_fn_1(lambda: f32) -> usize {
        ((-lambda).sqrt() * 300.0) as usize
    }

    fn some_bit_fn_2(lambda: f32) -> usize {
        1 + (some_bit_fn_1(lambda) & !7_usize)
    }

    struct Frame;

    struct PartEncoder1 {
        exp_calls: usize,
        enc_calls: usize,
    }

    impl PartEncoder1 {
        fn new(exp_calls: usize) -> Self {
            Self {
                exp_calls,
                enc_calls: 0,
            }
        }
    }

    impl BitStreamPartEncoder<Frame> for PartEncoder1 {
        fn encode(
            &mut self,
            _frame: &mut Frame,
            ba: &mut BitAllocHandler,
        ) -> Result<EncodeStatus, BitStreamEncodeError> {
            self.enc_calls += 1;
            ba.start(1000, -15.0, -1.0);
            Ok(EncodeStatus::Ok)
        }

        fn dump(&mut self, _bs: &mut BitStream) {
            assert_eq!(self.exp_calls, self.enc_calls);
        }

        fn consumption(&self) -> u32 {
            0
        }
    }

    struct PartEncoder2 {
        exp_calls: usize,
        enc_calls: usize,
        bits: usize,
        f: fn(f32) -> usize,
    }

    impl PartEncoder2 {
        fn new(exp_calls: usize, f: fn(f32) -> usize) -> Self {
            Self {
                exp_calls,
                enc_calls: 0,
                bits: 0,
                f,
            }
        }
    }

    impl BitStreamPartEncoder<Frame> for PartEncoder2 {
        fn encode(
            &mut self,
            _frame: &mut Frame,
            ba: &mut BitAllocHandler,
        ) -> Result<EncodeStatus, BitStreamEncodeError> {
            self.enc_calls += 1;
            let lambda = ba.cont();
            let bits = (self.f)(lambda);
            ba.submit(bits);
            self.bits = bits;
            Ok(EncodeStatus::Ok)
        }

        fn dump(&mut self, bs: &mut BitStream) {
            assert_eq!(self.exp_calls, self.enc_calls);
            for _ in 0..self.bits {
                bs.write(1, 1);
            }
        }

        fn consumption(&self) -> u32 {
            self.bits as u32
        }
    }

    struct PartEncoder3 {
        exp_calls: usize,
        enc_calls: usize,
    }

    impl PartEncoder3 {
        fn new(exp_calls: usize) -> Self {
            Self {
                exp_calls,
                enc_calls: 0,
            }
        }
    }

    impl BitStreamPartEncoder<Frame> for PartEncoder3 {
        fn encode(
            &mut self,
            _frame: &mut Frame,
            _ba: &mut BitAllocHandler,
        ) -> Result<EncodeStatus, BitStreamEncodeError> {
            self.enc_calls += 1;
            Ok(EncodeStatus::Ok)
        }

        fn dump(&mut self, _bs: &mut BitStream) {
            assert_eq!(self.exp_calls, self.enc_calls);
        }

        fn consumption(&self) -> u32 {
            0
        }
    }

    #[derive(Default)]
    struct PartEncoder4 {
        enc_calls: usize,
    }

    impl BitStreamPartEncoder<Frame> for PartEncoder4 {
        fn encode(
            &mut self,
            _frame: &mut Frame,
            _ba: &mut BitAllocHandler,
        ) -> Result<EncodeStatus, BitStreamEncodeError> {
            if self.enc_calls == 0 {
                self.enc_calls += 1;
                return Ok(EncodeStatus::Repeat);
            }

            Ok(EncodeStatus::Ok)
        }

        fn dump(&mut self, _bs: &mut BitStream) {
            assert_eq!(1, self.enc_calls);
        }

        fn consumption(&self) -> u32 {
            0
        }
    }

    #[test]
    fn simple_alloc() {
        let mut frame = Frame;
        let mut bs = BitStream::new();
        let mut encoder = BitStreamEncoder::new(vec![
            Box::new(PartEncoder1::new(1)),
            Box::new(PartEncoder2::new(8, some_bit_fn_1)),
            Box::new(PartEncoder3::new(1)),
        ]);

        encoder.run(&mut frame, &mut bs).unwrap();

        assert_eq!(1000, bs.size_in_bits());
    }

    #[test]
    fn alloc_with_repeat() {
        let mut frame = Frame;
        let mut bs = BitStream::new();
        let mut encoder = BitStreamEncoder::new(vec![
            Box::new(PartEncoder1::new(2)),
            Box::new(PartEncoder2::new(16, some_bit_fn_1)),
            Box::new(PartEncoder3::new(2)),
            Box::new(PartEncoder4::default()),
        ]);

        encoder.run(&mut frame, &mut bs).unwrap();

        assert_eq!(1000, bs.size_in_bits());
    }

    #[test]
    fn not_exact_alloc() {
        let mut frame = Frame;
        let mut bs = BitStream::new();
        let mut encoder = BitStreamEncoder::new(vec![
            Box::new(PartEncoder1::new(1)),
            Box::new(PartEncoder2::new(11, some_bit_fn_2)),
            Box::new(PartEncoder3::new(1)),
        ]);

        encoder.run(&mut frame, &mut bs).unwrap();

        assert_eq!(993, bs.size_in_bits());
    }
}
