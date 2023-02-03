use anyhow::Result;
use bytes::Bytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, PartialEq, Clone)]
pub struct NoSizeBytes(Bytes);

impl NoSizeBytes {
    pub fn new(data: Bytes) -> Self {
        Self(data)
    }

    pub fn into_bytes(self) -> Bytes {
        self.0
    }

    pub fn as_bytes(&self) -> Bytes {
        self.0.clone()
    }

    pub fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &u8> {
        self.0.iter()
    }
}

impl Serialize for NoSizeBytes {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for NoSizeBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = NoSizeBytes;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("Compact <ip=4><port=2> bytes")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(NoSizeBytes(Bytes::from(Vec::from(v))))
            }
        }

        deserializer.deserialize_byte_buf(Visitor {})
    }
}
