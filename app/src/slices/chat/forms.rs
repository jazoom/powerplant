use serde::Deserialize;

pub(super) const MAXIMUM_MESSAGE_BYTES: usize = 32_768;

#[derive(Deserialize)]
pub(super) struct ChatForm {
    #[serde(default)]
    pub(super) message: String,
}

impl ChatForm {
    pub(super) fn is_bounded(&self) -> bool {
        let message = self.message.trim();
        !message.is_empty() && message.len() <= MAXIMUM_MESSAGE_BYTES
    }
}

#[cfg(test)]
mod tests;
