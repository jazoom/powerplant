//! Shared Askama filters.

/// Read the response CSP nonce. Templates must not put the nonce on view models.
#[askama::filter_fn]
pub(crate) fn csp_nonce<'a>(_: &str, values: &'a dyn askama::Values) -> askama::Result<&'a str> {
    Ok(askama::get_value::<String>(values, "nonce")?.as_str())
}
