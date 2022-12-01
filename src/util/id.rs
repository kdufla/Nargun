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
