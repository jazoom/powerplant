use rand::rand_core::TryRng;
use rand::rngs::SysRng;

pub(crate) const WORKFLOW_ID_LENGTH: usize = 32;

const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct HexId([u8; 16]);

impl HexId {
    fn generate() -> Result<Self, IdError> {
        let mut bytes = [0u8; 16];
        SysRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| IdError::RandomUnavailable)?;
        Ok(Self(bytes))
    }

    fn parse(value: &str) -> Option<Self> {
        if value.len() != WORKFLOW_ID_LENGTH {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = decode_hex_byte(chunk[0], chunk[1])?;
        }
        Some(Self(bytes))
    }

    fn as_hex(&self) -> String {
        let mut out = String::with_capacity(WORKFLOW_ID_LENGTH);
        for byte in self.0 {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        out
    }
}

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub(crate) struct $name(HexId);

        impl $name {
            pub(crate) fn parse(value: &str) -> Option<Self> {
                HexId::parse(value).map(Self)
            }

            pub(crate) fn as_hex(&self) -> String {
                self.0.as_hex()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.as_hex())
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(stringify!($name))?;
                formatter.write_str("(")?;
                formatter.write_str(&self.as_hex())?;
                formatter.write_str(")")
            }
        }
    };
}

opaque_id!(WorkflowId);
opaque_id!(RunId);
opaque_id!(AttemptId);

impl RunId {
    pub(crate) fn generate() -> Result<Self, IdError> {
        Ok(Self(HexId::generate()?))
    }
}

impl AttemptId {
    pub(crate) fn generate() -> Result<Self, IdError> {
        Ok(Self(HexId::generate()?))
    }
}

#[derive(Debug)]
pub(crate) enum IdError {
    RandomUnavailable,
}

impl std::fmt::Display for IdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("system random source unavailable")
    }
}

impl std::error::Error for IdError {}

fn decode_hex_byte(high: u8, low: u8) -> Option<u8> {
    Some((decode_hex_nibble(high)? << 4) | decode_hex_nibble(low)?)
}

fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
