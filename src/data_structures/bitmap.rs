use crate::unsigned_ceil_div;
use anyhow::{bail, Result};

#[derive(Debug, Clone)]
pub struct Bitmap {
    data: Vec<u8>,
    len: usize,
}

impl Bitmap {
    pub fn new(len: usize) -> Self {
        let number_of_bytes_needed = unsigned_ceil_div!(len, 8);
        Self {
            data: vec![0; number_of_bytes_needed],
            len,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn replace_data(&mut self, new_data: &[u8]) -> Result<()> {
        if new_data.len() != self.data.len() {
            bail!(
                "piece count mismatch old={}, new={}",
                self.data.len(),
                new_data.len()
            );
        }

        for (i, d) in new_data.iter().enumerate() {
            self.data[i] = *d;
        }

        Ok(())
    }

    pub fn get(&self, i: usize) -> bool {
        let byte_idx = i / 8;
        let bit_offset = i % 8;

        let bit_getter = 0b1000_0000u8 as u8 >> bit_offset;

        bit_getter & self.data[byte_idx] > 0
    }

    pub fn change(&mut self, idx: usize, val: bool) -> bool {
        let byte_idx = idx / 8;
        let bit_offset = idx % 8;

        let bit_getter = 0b1000_0000u8 >> bit_offset;
        let bit_val = bit_getter & self.data[byte_idx];

        let bit_clearer = !bit_getter;
        let byte_without_target_bit = bit_clearer & self.data[byte_idx];

        let bit_setter = (val as u8) << (7 - bit_offset);

        self.data[byte_idx] = byte_without_target_bit | bit_setter;

        bit_val > 0
    }

    pub fn weight(&self) -> usize {
        self.data.iter().map(|x| x.count_ones()).sum::<u32>() as usize
    }

    pub fn iter(&self) -> Iter {
        Iter {
            data: self,
            cur_idx: 0,
            cur_byte: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Iter<'a> {
    data: &'a Bitmap,
    cur_idx: usize,
    cur_byte: u8,
}

impl<'a> Iterator for Iter<'a> {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur_idx == self.data.len {
            return None;
        }

        if self.cur_idx % 8 == 0 {
            self.cur_byte = self.data.data[self.cur_idx / 8];
        }

        let rv = 0b1000_0000 & self.cur_byte > 0;

        self.cur_byte = self.cur_byte << 1;
        self.cur_idx += 1;

        Some(rv)
    }
}

#[cfg(test)]
mod tests {
    use super::Bitmap;
    use crate::unsigned_ceil_div;

    #[test]
    fn setup() {
        let len = 67;
        let bitmap = Bitmap::new(len);

        assert_eq!(len, bitmap.len());

        assert_eq!(unsigned_ceil_div!(67, 8), bitmap.data.len());
    }

    #[test]
    fn change() {
        let mut bitmap = Bitmap::new(67);
        bitmap.data[7] = 0b0101_1010;

        assert_eq!(true, bitmap.change(7 * 8 + 1, false));
        assert_eq!(false, bitmap.change(7 * 8 + 7, true));

        assert_eq!(0b0001_1011, bitmap.data[7]);
    }

    #[test]
    fn weight() {
        let mut bitmap = Bitmap::new(21);

        let new_data = [0b0000_1111, 0b1110_0011, 0b1111_0000];
        bitmap.replace_data(&new_data).unwrap();

        assert_eq!(13, bitmap.weight())
    }

    #[test]
    fn replace() {
        let mut bitmap = Bitmap::new(21);

        let new_data = [0b0000_1111, 0b1110_0011, 0b1111_0000];

        bitmap.replace_data(&new_data).unwrap();

        for (idx, byte) in bitmap.data.iter().enumerate() {
            assert_eq!(new_data[idx], *byte);
        }
    }

    #[test]
    #[should_panic]
    fn replace_with_too_long() {
        let mut bitmap = Bitmap::new(21);

        let new_data = [0, 1, 2, 3, 4];

        bitmap.replace_data(&new_data).unwrap();
    }

    #[test]
    fn iter() {
        let mut bitmap = Bitmap::new(13);

        let new_data = [0b0000_1111, 0b1110_0000];
        bitmap.replace_data(&new_data).unwrap();

        let mut iter = bitmap.iter();

        assert_eq!(Some(false), iter.next());
        assert_eq!(Some(false), iter.next());
        assert_eq!(Some(false), iter.next());
        assert_eq!(Some(false), iter.next());
        assert_eq!(Some(true), iter.next());
        assert_eq!(Some(true), iter.next());
        assert_eq!(Some(true), iter.next());
        assert_eq!(Some(true), iter.next());

        assert_eq!(Some(true), iter.next());
        assert_eq!(Some(true), iter.next());
        assert_eq!(Some(true), iter.next());
        assert_eq!(Some(false), iter.next());
        assert_eq!(Some(false), iter.next());

        assert_eq!(None, iter.next());
    }
}
