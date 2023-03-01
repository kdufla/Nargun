use bytes::Bytes;
use openssl::sha;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    cmp::Ordering,
    fmt,
    ops::{BitXor, Sub},
};

pub const ID_LEN: usize = 20;
pub const ID_BIT_COUNT: usize = ID_LEN * 8;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct ID([u8; ID_LEN]);

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

    pub fn change_bit(&mut self, i: usize, v: bool) {
        let byte_idx = i / 8;
        let bit_offset = i % 8;

        let bit_clearer = !(0b1000_0000u8 >> bit_offset);
        let byte_without_target_bit = bit_clearer & self.0[byte_idx];

        // false is 0 and true is 1 (0b0000_0001), but I'm indexing left -> right
        // shift left 7 and shift right bit_offset (up to 7), ie 7 - bit_offset
        let bit_setter = (v as u8) << (7 - bit_offset);

        self.0[byte_idx] = byte_without_target_bit | bit_setter;
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

    fn bit_iter(&self) -> BitIter {
        BitIter {
            data: self.to_owned(),
            idx: 0,
        }
    }

    pub fn first_diff_bit_idx(&self, other: &Self) -> Option<usize> {
        for (idx, (l, r)) in self.bit_iter().zip(other.bit_iter()).enumerate() {
            if l != r {
                return Some(idx);
            }
        }

        None
    }

    pub fn randomize_after_bit(&mut self, i: usize) {
        if i >= ID_BIT_COUNT {
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

    pub fn bit_based_order<T>(
        &self,
        bit_idx: usize,
        first_if_true: T,
        first_if_false: T,
    ) -> (T, T) {
        if self.get_bit(bit_idx) {
            (first_if_true, first_if_false)
        } else {
            (first_if_false, first_if_true)
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

impl Serialize for ID {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_bytes(&self.0)
    }
}

impl Default for ID {
    fn default() -> Self {
        Self::new(rand::random())
    }
}

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

impl fmt::Debug for ID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rv = String::with_capacity(64);
        rv.push_str(&format!(" {:08b} ", self.0[0]));
        rv.push_str(&format!("{:08b} ", self.0[1]));

        for chunk in self.0.chunks(4) {
            rv.push(' ');
            for byte in chunk {
                rv.push_str(&format!("{byte:02X?}"));
            }
        }

        f.write_str(&rv[1..])
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
        let mut rv = [0u8; ID_LEN];

        for (res, (left, right)) in rv.iter_mut().zip(self.0.iter().zip(rhs.0.iter())) {
            *res = left ^ right;
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

impl From<Vec<u8>> for ID {
    fn from(vec: Vec<u8>) -> Self {
        if vec.len() != ID_LEN {
            panic!("Vec<u8>::len() must be {ID_LEN}");
        }

        ID(vec.try_into().unwrap())
    }
}

struct BitIter {
    data: ID,
    idx: usize,
}

impl Iterator for BitIter {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx < ID_LEN * 8 {
            let rv = self.data.get_bit(self.idx);
            self.idx += 1;
            Some(rv)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use tracing_test::traced_test;

    use super::{ID, ID_LEN};
    use std::cmp::Ordering;

    #[test]
    fn create() {
        let arr: [u8; ID_LEN] = rand::random();

        let id = ID::new(arr.to_owned());

        for (i, x) in arr.iter().enumerate() {
            assert_eq!(*x, id.0[i]);
        }
    }

    #[test]
    fn get_bit() {
        let mut id = ID::new(rand::random());

        id.0[13] = 0b0001_0000;
        assert!(!id.get_bit(13 * 8 + 1));
        assert!(id.get_bit(13 * 8 + 3));

        id.0[13] = 0b0100_1001;
        assert!(id.get_bit(13 * 8 + 1));
        assert!(!id.get_bit(13 * 8 + 3));
    }

    #[test]
    fn flip_bit() {
        let mut id = ID::new(rand::random());

        id.0[13] = 0b0000_1111;
        id.flip_bit(13 * 8 + 3);

        assert_eq!(0b0001_1111, id.0[13])
    }

    #[test]
    #[traced_test]
    fn change_bit() {
        let mut id = ID::new(rand::random());
        id.0[17] = 0b0101_1010;

        id.change_bit(17 * 8 + 1, false);
        id.change_bit(17 * 8 + 7, true);

        assert_eq!(0b0001_1011, id.0[17]);
    }

    #[test]
    fn cmp() {
        let mut id0 = ID::new(rand::random());
        let mut id1 = ID::new(rand::random());

        id0.0[0] = 0b1010_1010;
        id1.0[0] = 0b1010_1010;

        id0.0[1] = 0b1010_0101;
        id1.0[1] = 0b1010_1010;

        assert_eq!(Ordering::Equal, id0.cmp_first_n_bits(&id1, 7));
        assert_eq!(Ordering::Equal, id1.cmp_first_n_bits(&id0, 7));
        assert_eq!(Ordering::Equal, id0.cmp_first_n_bits(&id1, 10));

        assert_eq!(Ordering::Greater, id1.cmp_first_n_bits(&id0, 15));
        assert_eq!(Ordering::Less, id0.cmp_first_n_bits(&id1, 14));
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
    fn eq() {
        let arr: [u8; ID_LEN] = rand::random();
        let arr2 = arr;

        let id = ID::new(arr);
        let id2 = ID::new(arr2);

        assert_eq!(id, id2);
    }

    #[test]
    fn ne() {
        let arr: [u8; ID_LEN] = rand::random();

        let mut arr2 = arr;
        arr2[9] = arr2[9].wrapping_add(1);

        let id = ID::new(arr);
        let id2 = ID::new(arr2);

        assert_ne!(id, id2);
    }

    #[test]
    fn ord() {
        let arr = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let id = ID::new(arr);

        let eq_arr = arr;
        let eq_id = ID::new(eq_arr);

        let mut less_arr = arr;
        less_arr[9] -= 1;
        let less_id = ID::new(less_arr);

        let mut greater_arr = arr;
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
}
