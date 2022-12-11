use std::{
    cmp::Ordering,
    ops::{BitXor, Sub},
};

#[derive(Debug, Clone, Hash)]
pub struct ID(pub [u8; 20]); // TODO this should not be pub

impl ID {
    pub fn new(id_array: [u8; 20]) -> Self {
        Self(id_array)
    }

    pub fn get_bit(&self, i: usize) -> bool {
        let byte_idx = i / 8;
        let bit_offset = i % 8;

        let bit_filter = 0b1000_0000u8 >> bit_offset;

        bit_filter & self.0[byte_idx] > 0
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

impl BitXor for ID {
    type Output = Self;

    fn bitxor(self, rhs: Self) -> Self::Output {
        let mut rv = [0 as u8; 20];

        for i in 0..self.0.len() {
            rv[i] = self.0[i] ^ rhs.0[i];
        }

        Self(rv)
    }
}

impl Sub for ID {
    type Output = Self;

    fn sub(self, other: Self) -> Self::Output {
        self ^ other
    }
}

#[cfg(test)]
mod id_tests {
    use std::cmp::Ordering;

    use crate::constants::ID_LEN;

    use super::ID;

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
        let left = [
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
        ];
        let right = [
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
        ];
        let res = [
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
        ];

        assert_eq!(ID::new(left) ^ ID::new(right), ID::new(res));
    }

    #[test]
    fn sub() {
        let left: [u8; ID_LEN] = rand::random();
        let right: [u8; ID_LEN] = rand::random();

        assert_eq!(
            ID::new(left.clone()) ^ ID::new(right.clone()),
            ID::new(left.clone()) - ID::new(right.clone())
        );

        assert_eq!(
            ID::new(left.clone()) ^ ID::new(right.clone()),
            ID::new(right) - ID::new(left)
        );
    }
}
