use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Boolean flags for user preferences
    #[derive(Debug, Clone, Copy)]
    pub struct Capability: u64 {
        const S3 = 1;
        const PUSH_NOTIFICATIONS = 1 << 1;
    }
}

impl Default for Capability {
    fn default() -> Self {
        Self::empty()
    }
}

impl Serialize for Capability {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.bits())
    }
}

impl<'de> Deserialize<'de> for Capability {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let flags = u64::deserialize(deserializer)?;
        Ok(Self::from_bits(flags).unwrap_or_default())
    }
}
