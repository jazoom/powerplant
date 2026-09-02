use super::*;

#[test]
fn kind_and_payload_changes_create_different_artefact_hashes() {
    let (left, _, left_hash) = encode_plan("# Plan\n", None).expect("left");
    let (_right, _, right_hash) = encode_plan("# Other\n", None).expect("right");
    assert_ne!(left_hash, right_hash);
    let object = ObjectHash::of(&left);
    assert_ne!(object.as_str(), left_hash.as_str());
    let review = encode_review(
        CandidateHash::of(b"tree"),
        ReviewVerdict::Approved,
        "# Plan\n",
        None,
    )
    .expect("review");
    assert_ne!(review.2, left_hash);
    assert_ne!(review.1, ObjectHash::of(&left));
}

#[test]
fn stable_order_creates_the_same_hash() {
    let (_, _, first) = encode_plan("Hello\n", None).expect("first");
    let (_, _, second) = encode_plan("Hello\r\n", None).expect("second");
    assert_eq!(first, second);
    let (bytes, _, _) = encode_plan("caf\u{e9}", None).expect("unicode");
    assert!(
        String::from_utf8(bytes)
            .expect("utf8")
            .contains("caf\u{e9}")
    );
}

#[test]
fn payload_bounds_and_nul_text_are_rejected() {
    assert_eq!(
        encode_plan("bad\u{0000}text", None).err(),
        Some(PayloadError::Text)
    );
    let huge = "a".repeat(MAXIMUM_PLAN_BYTES + 1);
    assert_eq!(encode_plan(&huge, None).err(), Some(PayloadError::Bound));
    assert_eq!(
        encode_plan("use sk-secret", Some("sk-secret")).err(),
        Some(PayloadError::Credential)
    );
    assert!(CandidateHash::parse("sha256:zz").is_none());
}

#[test]
fn duplicate_fields_are_rejected() {
    let bytes = br#"{"format-version":1,"markdown":"a","markdown":"b"}"#;
    assert_eq!(
        parse_typed_payload(ArtefactKind::Plan, bytes).err(),
        Some(PayloadError::DuplicateField)
    );
}
