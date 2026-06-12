#[derive(Debug, Clone)]
pub struct DelayBuffer<T, const N: usize, const S: usize>
where
    T: Copy + Default,
{
    buffer: [[[T; S]; 2]; N],
}

impl<T, const N: usize, const S: usize> DelayBuffer<T, N, S>
where
    T: Copy + Default,
{
    pub fn new() -> Self {
        Self {
            buffer: [[[T::default(); S]; 2]; N],
        }
    }

    pub fn shift(&mut self, erase: bool) {
        for row in &mut self.buffer {
            row[0] = row[1];
            if erase {
                row[1].fill(T::default());
            }
        }
    }

    pub fn first(&self, i: usize) -> &[T] {
        &self.buffer[i][0]
    }

    pub fn first_mut(&mut self, i: usize) -> &mut [T] {
        &mut self.buffer[i][0]
    }

    pub fn second(&self, i: usize) -> &[T] {
        &self.buffer[i][1]
    }

    pub fn second_mut(&mut self, i: usize) -> &mut [T] {
        &mut self.buffer[i][1]
    }
}

impl<T, const N: usize, const S: usize> Default for DelayBuffer<T, N, S>
where
    T: Copy + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_copies_second_to_first_and_erases_by_default() {
        let mut buf = DelayBuffer::<i32, 2, 3>::new();
        buf.second_mut(0).copy_from_slice(&[1, 2, 3]);
        buf.second_mut(1).copy_from_slice(&[4, 5, 6]);

        buf.shift(true);

        assert_eq!([1, 2, 3], buf.first(0));
        assert_eq!([4, 5, 6], buf.first(1));
        assert_eq!([0, 0, 0], buf.second(0));
        assert_eq!([0, 0, 0], buf.second(1));
    }

    #[test]
    fn shift_can_keep_second_half() {
        let mut buf = DelayBuffer::<i32, 1, 2>::new();
        buf.second_mut(0).copy_from_slice(&[7, 8]);

        buf.shift(false);

        assert_eq!([7, 8], buf.first(0));
        assert_eq!([7, 8], buf.second(0));
    }
}
