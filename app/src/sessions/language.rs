use axum::http::{HeaderMap, header};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BrowserLanguage(String);

impl BrowserLanguage {
    pub(crate) fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let values = headers.get_all(header::ACCEPT_LANGUAGE);
        if values.iter().map(|value| value.len()).sum::<usize>() > 1024 {
            return None;
        }
        let mut preferred = None;
        let mut highest_quality = 0;
        for value in values {
            for entry in value.to_str().ok()?.split(',') {
                let mut parts = entry.trim().split(';');
                let tag = parts.next()?.trim();
                let mut subtags = tag.split('-');
                let primary = subtags.next()?;
                if tag.len() > 63
                    || !(2..=8).contains(&primary.len())
                    || !primary.bytes().all(|byte| byte.is_ascii_alphabetic())
                    || !subtags.all(|part| {
                        (1..=8).contains(&part.len())
                            && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
                    })
                {
                    continue;
                }
                let quality = match parts.next() {
                    None => 1000,
                    Some(parameter) => {
                        let Some(quality) = parameter.trim().strip_prefix("q=").and_then(quality)
                        else {
                            continue;
                        };
                        quality
                    }
                };
                if parts.next().is_none() && quality > highest_quality {
                    preferred = Some(Self(tag.to_owned()));
                    highest_quality = quality;
                }
            }
        }
        preferred
    }

    pub(crate) fn append_instructions(&self, instructions: &mut String) {
        if !instructions.is_empty() {
            instructions.push_str("\n\n");
        }
        instructions.push_str(&format!(
            "The browser's preferred locale is {}. If the language or regional variant is ambiguous, use this locale. Explicit language requests and the conversation's established language take priority over this fallback.",
            self.0
        ));
    }
}

fn quality(raw: &str) -> Option<u16> {
    let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    if fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    match whole {
        "1" if fraction.bytes().all(|byte| byte == b'0') => Some(1000),
        "0" => Some(
            fraction
                .bytes()
                .fold(0, |value, byte| value * 10 + u16::from(byte - b'0'))
                * 10u16.pow(3 - fraction.len() as u32),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
