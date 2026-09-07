use serde::{Deserialize, Serialize};
use time::{Date, Month};

use super::catalogue::Model;

pub(crate) const TITLE_INPUT_TOKENS: u64 = 2_000;
pub(crate) const TITLE_OUTPUT_TOKENS: u64 = 128;
const TITLE_COST_CEILING: f64 = 0.001;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BackgroundMetadata {
    pub(super) release_date: String,
    pub(super) input_cost: f64,
    pub(super) output_cost: f64,
    pub(super) output_limit: u64,
}

impl BackgroundMetadata {
    pub(super) fn valid(&self) -> bool {
        release_date(&self.release_date).is_some()
            && self.input_cost.is_finite()
            && self.output_cost.is_finite()
            && self.input_cost > 0.0
            && self.output_cost > 0.0
            && self.output_limit >= TITLE_OUTPUT_TOKENS
    }

    fn estimate(&self) -> f64 {
        (self.input_cost * TITLE_INPUT_TOKENS as f64
            + self.output_cost * TITLE_OUTPUT_TOKENS as f64)
            / 1_000_000.0
    }
}

pub(super) fn select(models: &[Model], today: Date) -> Option<&Model> {
    let earliest = Date::from_calendar_date(today.year() - 1, today.month(), today.day())
        .or_else(|_| Date::from_calendar_date(today.year() - 1, today.month(), 28))
        .ok()?;
    let candidates: Vec<_> = models
        .iter()
        .filter_map(|model| {
            let metadata = model.background.as_ref()?;
            let date = release_date(&metadata.release_date)?;
            let cost = metadata.estimate();
            (metadata.valid()
                && model.limit.context >= TITLE_INPUT_TOKENS + TITLE_OUTPUT_TOKENS
                && date >= earliest
                && date <= today
                && cost <= TITLE_COST_CEILING)
                .then_some((model, date, cost))
        })
        .collect();
    let cheapest = candidates
        .iter()
        .map(|(_, _, cost)| *cost)
        .reduce(f64::min)?;
    candidates
        .into_iter()
        .filter(|(_, _, cost)| *cost <= cheapest * 2.0)
        .max_by(
            |(left, left_date, left_cost), (right, right_date, right_cost)| {
                left_date
                    .cmp(right_date)
                    .then_with(|| right_cost.total_cmp(left_cost))
                    .then_with(|| right.id.cmp(&left.id))
            },
        )
        .map(|(model, _, _)| model)
}

// Month-only dates use the first day. Unknown days must not extend eligibility.
fn release_date(raw: &str) -> Option<Date> {
    if !matches!(raw.len(), 7 | 10) {
        return None;
    }
    let mut parts = raw.split('-');
    let year = parts.next()?;
    let month = parts.next()?;
    let day = parts.next().unwrap_or("01");
    if year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || parts.next().is_some()
        || ![year, month, day]
            .iter()
            .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    Date::from_calendar_date(
        year.parse().ok()?,
        Month::try_from(month.parse::<u8>().ok()?).ok()?,
        day.parse().ok()?,
    )
    .ok()
}

pub(super) fn unsuitable(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    [
        ":free",
        "preview",
        "experimental",
        "realtime",
        "multi-agent",
        "embedding",
        "safeguard",
        "guard",
        "moderation",
        "deep-research",
        "translate",
    ]
    .iter()
    .any(|part| id.contains(part))
}

#[cfg(test)]
mod tests;
