use anyhow::{anyhow, Result};
use std::sync::{Arc, Mutex as StdMutex};

#[derive(Debug, Clone, Hash)]
pub struct ID(pub [u8; 20]);

impl PartialEq for ID {
    fn eq(&self, other: &Self) -> bool {
        for (i, byte) in self.0.iter().enumerate() {
            if *byte != other.0[i] {
                return false;
            }
        }

        true
    }
}

#[macro_export]
macro_rules! unsigned_ceil_div {
    ($x:expr, $y:expr) => {{
        1 + (($x - 1) / $y)
    }};
}

#[derive(Debug, Clone)]
pub struct Bitmap {
    data: Arc<StdMutex<Vec<u8>>>,
    len: usize,
}

impl Bitmap {
    pub fn new(n: u32) -> Bitmap {
        let number_of_bytes_needed = (f64::from(n) / 8.0).ceil() as usize;
        Bitmap {
            data: Arc::new(StdMutex::new(vec![0; number_of_bytes_needed])),
            len: n as usize,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn replace_data(&self, new_data: &[u8]) -> Result<()> {
        let mut data = self.data.lock().unwrap();

        if new_data.len() == data.len() {
            for (i, d) in new_data.iter().enumerate() {
                data[i] = *d;
            }

            Ok(())
        } else {
            Err(anyhow!(
                "piece count mismatch old={}, new={}",
                data.len(),
                new_data.len()
            ))
        }
    }

    pub fn get(&self, i: usize) -> bool {
        let byte_idx = i / 8;
        let bit_offset = i % 8;

        let bits = 128 as u8 >> bit_offset;

        bits & self.data.lock().unwrap()[byte_idx] > 0
    }

    pub fn set(&self, idx: usize, val: bool) {
        let mut data = self.data.lock().unwrap();

        let byte_idx = idx / 8;
        let bit_offset = idx % 8;

        if val {
            let bits = 0b1000_0000 as u8 >> bit_offset;

            data[byte_idx] = data[byte_idx] | bits;
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

            data[byte_idx] = data[byte_idx] & clear_bit[bit_offset];
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
