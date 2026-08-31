use super::{decode, encode};

#[test]
fn encode_uses_lowercase_digits() {
    assert_eq!(encode(&[]), "");
    assert_eq!(encode(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
}

#[test]
fn decode_accepts_exact_lowercase_length() {
    assert_eq!(decode::<4>("000fa0ff"), Some([0x00, 0x0f, 0xa0, 0xff]));
    assert_eq!(decode::<32>(&"cd".repeat(32)), Some([0xcd; 32]));
}

#[test]
fn decode_rejects_uppercase() {
    assert_eq!(decode::<1>("A0"), None);
    assert_eq!(decode::<1>("0A"), None);
}

#[test]
fn decode_rejects_odd_and_wrong_lengths() {
    assert_eq!(decode::<2>("abc"), None);
    assert_eq!(decode::<2>("ab"), None);
    assert_eq!(decode::<2>("abcdef"), None);
}

#[test]
fn decode_rejects_non_hexadecimal_bytes() {
    assert_eq!(decode::<1>("g0"), None);
    assert_eq!(decode::<1>("0/"), None);
}

#[test]
fn encode_and_decode_round_trip_fixed_arrays() {
    let bytes = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
    let hex = encode(&bytes);
    assert_eq!(hex, "0123456789abcdef");
    assert_eq!(decode::<8>(&hex), Some(bytes));
}
