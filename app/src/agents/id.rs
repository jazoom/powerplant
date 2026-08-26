use rand::rand_core::TryRng;
use rand::rngs::SysRng;

pub(crate) const AGENT_ID_LENGTH: usize = 32;

const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct AgentId([u8; 16]);

impl AgentId {
    pub(crate) fn generate() -> Result<Self, AgentIdError> {
        let mut bytes = [0u8; 16];
        SysRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| AgentIdError::RandomUnavailable)?;
        Ok(Self(bytes))
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        if value.len() != AGENT_ID_LENGTH {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = decode_hex_byte(chunk[0], chunk[1])?;
        }
        Some(Self(bytes))
    }

    pub(crate) fn as_hex(&self) -> String {
        let mut out = String::with_capacity(AGENT_ID_LENGTH);
        for byte in self.0 {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        out
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.as_hex())
    }
}

impl std::fmt::Debug for AgentId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentId(")?;
        formatter.write_str(&self.as_hex())?;
        formatter.write_str(")")
    }
}

#[derive(Debug)]
pub(crate) enum AgentIdError {
    RandomUnavailable,
}

impl std::fmt::Display for AgentIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("system random source unavailable")
    }
}

impl std::error::Error for AgentIdError {}

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
