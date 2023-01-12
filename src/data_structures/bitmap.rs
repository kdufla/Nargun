use anyhow::{bail, Result};
use std::sync::{Arc, Mutex as StdMutex};

#[derive(Debug, Clone)]
pub struct Bitmap {
    data: Arc<StdMutex<Vec<u8>>>,
    len: usize,
}

impl Bitmap {
    pub fn new(n: u32) -> Self {
        let number_of_bytes_needed = (f64::from(n) / 8.0).ceil() as usize;
        Self {
            data: Arc::new(StdMutex::new(vec![0; number_of_bytes_needed])),
            len: n as usize,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn replace_data(&self, new_data: &[u8]) -> Result<()> {
        let mut data = self.data.lock().unwrap();

        if new_data.len() != data.len() {
            bail!(
                "piece count mismatch old={}, new={}",
                data.len(),
                new_data.len()
            );
        }

        for (i, d) in new_data.iter().enumerate() {
            data[i] = *d;
        }

        Ok(())
    }

    pub fn get(&self, i: usize) -> bool {
        let byte_idx = i / 8;
        let bit_offset = i % 8;

        let bits = 128 as u8 >> bit_offset;

        bits & self.data.lock().unwrap()[byte_idx] > 0
    }

    pub fn change(&self, idx: usize, val: bool) {
        let mut data = self.data.lock().unwrap();

        let byte_idx = idx / 8;
        let bit_offset = idx % 8;

        let mut bits = 0b1000_0000 as u8 >> bit_offset;

        if val {
            data[byte_idx] = data[byte_idx] | bits;
        } else {
            bits = !bits;
            data[byte_idx] = data[byte_idx] & bits;
        }
    }

    pub fn weight(&self) -> u32 {
        self.data
            .lock()
            .unwrap()
            .iter()
            .fold(0, |weight, x| weight + x.count_ones())
    }
}

#[cfg(test)]
mod tests {
    use crate::unsigned_ceil_div;

    use super::Bitmap;

    #[test]
    fn setup() {
        let len = 67;
        let bm = Bitmap::new(len as u32);

        assert_eq!(len, bm.len());

        let data = bm.data.lock().unwrap();
        assert_eq!(unsigned_ceil_div!(67, 8), data.len());
    }

    #[test]
    fn change() {
        let bm = Bitmap::new(67);
        bm.data.lock().unwrap()[7] = 0b0101_1010;

        bm.change(7 * 8 + 1, false);
        bm.change(7 * 8 + 7, true);

        assert_eq!(0b0001_1011, bm.data.lock().unwrap()[7]);
    }

    #[test]
    fn replace() {
        let bm = Bitmap::new(21);

        let new_data = [0b0000_1111, 0b1110_0011, 0b1111_0000];

        bm.replace_data(&new_data).unwrap();

        let data = bm.data.lock().unwrap();

        for (idx, byte) in data.iter().enumerate() {
            assert_eq!(new_data[idx], *byte);
        }
    }

    #[test]
    #[should_panic]
    fn replace_with_too_long() {
        let bm = Bitmap::new(21);

        let new_data = [0, 1, 2, 3, 4];

        bm.replace_data(&new_data).unwrap();
    }
}
