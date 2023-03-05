use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, PartialEq, Clone, Eq, Hash)]
pub struct SerializableBuf(Vec<u8>);

impl SerializableBuf {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }

    pub fn iter(&self) -> impl Iterator<Item = &u8> {
        self.0.iter()
    }
}

impl<T> From<T> for SerializableBuf
where
    T: Into<Vec<u8>>,
{
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

impl IntoIterator for SerializableBuf {
    type Item = u8;

    type IntoIter = std::vec::IntoIter<u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl AsRef<[u8]> for SerializableBuf {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Default for SerializableBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl Serialize for SerializableBuf {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for SerializableBuf {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = SerializableBuf;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("Compact <ip=4><port=2> bytes")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SerializableBuf(Vec::from(v)))
            }
        }

        deserializer.deserialize_byte_buf(Visitor {})
    }
}
