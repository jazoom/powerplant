use super::{TOKEN_LENGTH, ValidatedToken, generate};

#[test]
fn generated_token_round_trips() {
    let token = generate().unwrap();
    let parsed = ValidatedToken::parse(token.raw().as_str()).unwrap();
    assert_eq!(parsed.as_str().len(), TOKEN_LENGTH);
    assert_eq!(format!("{:?}", token.raw()), "ValidatedToken(<redacted>)");
    assert!(!format!("{:?}", token.raw()).contains(token.raw().as_str()));
}

#[test]
fn rejects_non_canonical_tokens() {
    assert!(ValidatedToken::parse("").is_none());
    assert!(ValidatedToken::parse("short").is_none());
    assert!(ValidatedToken::parse(&format!("{}=", "A".repeat(TOKEN_LENGTH - 1))).is_none());
}
