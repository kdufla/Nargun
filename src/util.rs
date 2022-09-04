use anyhow::{anyhow, Result};

#[derive(Debug)]
pub struct Bitmap {
    data: Vec<u8>,
    len: usize,
}

impl Bitmap {
    pub fn new(n: u32) -> Bitmap {
        let number_of_bytes_needed = (f64::from(n) / 8.0).ceil() as usize;
        Bitmap {
            data: vec![0; number_of_bytes_needed],
            len: n as usize,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn replace_data(&mut self, data: &[u8]) -> Result<()> {
        if data.len() == self.data.len() {
            for (i, d) in data.iter().enumerate() {
                self.data[i] = *d;
            }
            Ok(())
        } else {
            Err(anyhow!("piece count mismatch"))
        }
    }

    pub fn get(&self, i: usize) -> bool {
        let byte_idx = i / 8;
        let bit_offset = i % 8;

        let bits = 128 as u8 >> bit_offset;

        bits & self.data[byte_idx] > 0
    }

    pub fn set(&mut self, i: usize, v: bool) {
        let byte_idx = i / 8;
        let bit_offset = i % 8;

        if v {
            let bits = 0b1000_0000 as u8 >> bit_offset;

            self.data[byte_idx] = self.data[byte_idx] | bits;
        } else {
            let clear_bit = [
                0b0111_1111,
                0b1011_1111,
                0b1101_1111,
                0b1110_1111,
                0b1111_0111,
                0b1111_1011,
                0b1111_1101,
                0b1111_1110,
            ];

            self.data[byte_idx] = self.data[byte_idx] & clear_bit[bit_offset];
        }
    }

    pub fn weight(&self) -> u32 {
        self.data
            .iter()
            .fold(0, |weight, x| weight + x.count_ones())
    }
}
