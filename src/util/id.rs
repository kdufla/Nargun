use crate::constants::ID_LEN;
use bytes::Bytes;
use openssl::sha;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    cmp::Ordering,
    fmt,
    ops::{BitXor, Sub},
};

#[derive(Clone, Hash)]
pub struct ID([u8; ID_LEN]);

impl<'de> Deserialize<'de> for ID {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = [u8; ID_LEN];

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("byte string")
            }
            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(v.try_into().unwrap())
            }
        }
        Ok(ID(deserializer.deserialize_byte_buf(Visitor {})?))
    }
}

impl Serialize for ID {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_bytes(&self.0)
    }
}

impl ID {
    pub fn new(id_array: [u8; ID_LEN]) -> Self {
        Self(id_array)
    }

    pub fn get_bit(&self, i: usize) -> bool {
        let byte_idx = i / 8;
        let bit_offset = i % 8;

        let bit_filter = 0b1000_0000u8 >> bit_offset;

        (bit_filter & self.0[byte_idx]) > 0
    }

    pub fn flip_bit(&mut self, i: usize) {
        let byte_idx = i / 8;
        let bit_offset = i % 8;

        let bit_filter = 0b1000_0000u8 >> bit_offset;

        self.0[byte_idx] ^= bit_filter;
    }

    pub fn cmp_first_n_bits(&self, other: &Self, n: usize) -> Ordering {
        let byte_idx = n / 8;
        let bit_offset = n % 8;

        for (self_byte, other_byte) in self.0.iter().zip(other.0.iter()).take(byte_idx) {
            if self_byte.cmp(other_byte) != Ordering::Equal {
                return self_byte.cmp(other_byte);
            }
        }

        for i in 0..bit_offset {
            let cur_idx = byte_idx * 8 + i;

            if self.get_bit(cur_idx).cmp(&other.get_bit(cur_idx)) != Ordering::Equal {
                return self.get_bit(cur_idx).cmp(&other.get_bit(cur_idx));
            }
        }

        Ordering::Equal
    }

    pub fn randomize_after_bit(&mut self, i: usize) {
        if i >= ID_LEN * 8 {
            return;
        }

        let first_byte_idx = i / 8 + 1;

        let mut rng = thread_rng();

        for byte in &mut self.0[first_byte_idx..] {
            *byte = rng.gen();
        }

        for bit_idx in (i + 1)..(first_byte_idx * 8) {
            if rng.gen() {
                self.flip_bit(bit_idx);
            }
        }
    }

    pub fn left_or_right_by_depth<T>(&self, depth: usize, left: T, right: T) -> T {
        if self.get_bit(depth) {
            right
        } else {
            left
        }
    }

    pub fn hash_as_bytes(&self, secret: &[u8]) -> Bytes {
        let mut hasher = sha::Sha1::new();
        hasher.update(self.as_byte_ref());
        hasher.update(secret);
        Bytes::from(Vec::from(hasher.finish()))
    }

    pub fn as_byte_ref(&self) -> &[u8] {
        self.0.as_ref()
    }

    pub fn weight(&self) -> usize {
        // this is not necessary, I wanted to use unsafe
        let transmuted_bytes = unsafe { std::mem::transmute::<[u8; 20], [u32; 5]>(self.0) };
        (hamming_weight(transmuted_bytes[0])
            + hamming_weight(transmuted_bytes[1])
            + hamming_weight(transmuted_bytes[2])
            + hamming_weight(transmuted_bytes[3])
            + hamming_weight(transmuted_bytes[4])) as usize
    }
}

fn hamming_weight(mut x: u32) -> u32 {
    x -= (x >> 1) & 0x55555555;
    x = (x & 0x33333333) + ((x >> 2) & 0x33333333);
    x = (x + (x >> 4)) & 0x0f0f0f0f;
    (x.wrapping_mul(0x01010101)) >> 24
}

impl fmt::Debug for ID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rv = String::with_capacity(64);
        rv.push_str(&format!(" {:08b} ", self.0[0]));
        rv.push_str(&format!("{:08b} ", self.0[1]));

        for chunk in self.0.chunks(4) {
            rv.push(' ');
            for byte in chunk {
                rv.push_str(&format!("{:02X?}", byte));
            }
        }

        f.write_str(&rv[1..])
    }
}

impl Eq for ID {}

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

impl PartialOrd for ID {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ID {
    fn cmp(&self, other: &Self) -> Ordering {
        for (self_byte, other_byte) in self.0.iter().zip(other.0.iter()) {
            if self_byte.cmp(other_byte) != Ordering::Equal {
                return self_byte.cmp(other_byte);
            }
        }

        Ordering::Equal
    }
}

impl<'a, 'b> BitXor<&'b ID> for &'a ID {
    type Output = ID;

    fn bitxor(self, rhs: &'b ID) -> Self::Output {
        let mut rv = [0 as u8; ID_LEN];

        for i in 0..self.0.len() {
            rv[i] = self.0[i] ^ rhs.0[i];
        }

        ID(rv)
    }
}

impl<'a, 'b> Sub<&'b ID> for &'a ID {
    type Output = ID;

    fn sub(self, other: &'b ID) -> Self::Output {
        self ^ other
    }
}

#[cfg(test)]
mod tests {
    use super::{ID, ID_LEN};
    use std::cmp::Ordering;

    #[test]
    fn create() {
        let arr: [u8; ID_LEN] = rand::random();
        let arr_clone = arr.clone();

        let id = ID::new(arr_clone);

        for (i, x) in arr.iter().enumerate() {
            assert_eq!(*x, id.0[i]);
        }
    }

    #[test]
    fn eq() {
        let arr: [u8; ID_LEN] = rand::random();
        let arr2 = arr.clone();

        let id = ID::new(arr);
        let id2 = ID::new(arr2);

        assert_eq!(id, id2);
    }

    #[test]
    fn ne() {
        let arr: [u8; ID_LEN] = rand::random();

        let mut arr2 = arr.clone();
        arr2[9] = arr2[9].wrapping_add(1);

        let id = ID::new(arr);
        let id2 = ID::new(arr2);

        assert_ne!(id, id2);
    }

    #[test]
    fn ord() {
        let arr = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let id = ID::new(arr);

        let eq_arr = arr.clone();
        let eq_id = ID::new(eq_arr);

        let mut less_arr = arr.clone();
        less_arr[9] -= 1;
        let less_id = ID::new(less_arr);

        let mut greater_arr = arr.clone();
        greater_arr[9] += 1;
        let greater_id = ID::new(greater_arr);

        assert_eq!(eq_id.cmp(&id), Ordering::Equal);
        assert_eq!(less_id.cmp(&id), Ordering::Less);
        assert_eq!(greater_id.cmp(&id), Ordering::Greater);
    }

    #[test]
    fn xor() {
        let left = ID::new([
            0,
            0,
            0,
            0,
            0,
            0b0000_1111,
            0xff,
            0xff,
            0xff,
            0xff,
            0xff,
            0xff,
            0xff,
            0xff,
            0xff,
            0b1010_1010,
            0,
            0xff,
            0,
            0xff,
        ]);
        let right = ID::new([
            0xff,
            0xff,
            0,
            0,
            0,
            0b1010_1010,
            0,
            0xff,
            0,
            0xff,
            0,
            0,
            0,
            0,
            0,
            0b1110_0011,
            0,
            0,
            0,
            0,
        ]);
        let res = ID::new([
            0xff,
            0xff,
            0,
            0,
            0,
            0b1010_0101,
            0xff,
            0,
            0xff,
            0,
            0xff,
            0xff,
            0xff,
            0xff,
            0xff,
            0b0100_1001,
            0,
            0xff,
            0,
            0xff,
        ]);

        assert_eq!(&left ^ &right, res);
    }

    #[test]
    fn sub() {
        let left = ID::new(rand::random());
        let right = ID::new(rand::random());

        assert_eq!(&left ^ &right, &left - &right);
        assert_eq!(&left ^ &right, &right - &left);
    }

    #[test]
    fn weight() {
        assert_eq!(
            51,
            ID::new([
                1, 1, 1, 1, 1, 0xFF, 0xFF, 0, 0x11, 0xFF, 0xFF, 0xFF, 1, 0, 0, 0, 0, 0, 1, 5,
            ])
            .weight()
        );
    }

    #[test]
    fn randomize() {
        let mut id = ID::new([0; 20]);

        for _ in 0..64 {
            id.randomize_after_bit(8 + 2);
            assert!(id.as_byte_ref()[1] <= 0b0001_1111);
        }

        assert!(id.weight() > 0);
    }
}
