use std::io::{self, Write};

pub struct YamlLog<W: Write> {
    out: W,
}

impl<W: Write> YamlLog<W> {
    pub fn new(out: W) -> Self {
        Self { out }
    }

    pub fn write_float_seq(&mut self, values: &[f32], precision: usize) -> io::Result<()> {
        write_float_seq(&mut self.out, values, precision)
    }

    pub fn into_inner(self) -> W {
        self.out
    }
}

pub fn write_float_seq<W: Write + ?Sized>(
    out: &mut W,
    values: &[f32],
    precision: usize,
) -> io::Result<()> {
    out.write_all(b"[")?;
    for (i, value) in values.iter().enumerate() {
        if i != 0 {
            out.write_all(b", ")?;
        }
        write!(out, "{value:.precision$}")?;
    }
    out.write_all(b"]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_compact_float_sequence_with_default_cpp_style_precision() {
        let mut out = Vec::new();
        write_float_seq(&mut out, &[0.12, 1.23456, -2.0], 4).unwrap();
        assert_eq!("[0.1200, 1.2346, -2.0000]", String::from_utf8(out).unwrap());
    }

    #[test]
    fn yaml_log_owns_and_returns_writer() {
        let mut log = YamlLog::new(Vec::new());
        log.write_float_seq(&[1.0, 0.5], 2).unwrap();
        assert_eq!("[1.00, 0.50]", String::from_utf8(log.into_inner()).unwrap());
    }
}
