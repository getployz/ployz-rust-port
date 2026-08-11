use std::str::FromStr;

use ployz_internal_secret::{Secret, new, new_id, random_alphanumeric};

#[test]
fn from_hex_string_parses_empty_lowercase_and_uppercase() {
    let empty = Secret::from_hex_string("").expect("empty hexadecimal should parse");
    let lowercase = Secret::from_hex_string("00abff").expect("lowercase hexadecimal should parse");
    let uppercase = Secret::from_hex_string("00ABFF").expect("uppercase hexadecimal should parse");

    assert!(empty.is_empty());
    assert_eq!(lowercase.as_bytes(), &[0x00, 0xab, 0xff]);
    assert_eq!(uppercase, lowercase);
}

#[test]
fn from_hex_string_rejects_invalid_input_with_context() {
    let odd_length = Secret::from_hex_string("abc").expect_err("odd-length input must fail");
    let non_hex = Secret::from_hex_string("zz").expect_err("non-hex input must fail");

    assert!(
        odd_length
            .to_string()
            .starts_with("invalid hex-encoded secret: ")
    );
    assert!(
        non_hex
            .to_string()
            .starts_with("invalid hex-encoded secret: ")
    );
}

#[test]
fn text_formats_are_lowercase_hexadecimal() {
    let secret = Secret::from_hex_string("00ABFF").expect("valid hexadecimal should parse");

    assert_eq!(secret.to_hex_string(), "00abff");
    assert_eq!(secret.to_string(), "00abff");
    assert_eq!(
        Secret::from_str("00abff").expect("FromStr should parse valid hexadecimal"),
        secret
    );
}

#[test]
fn serde_uses_the_same_text_representation() {
    let secret = Secret::from_hex_string("00ABFF").expect("valid hexadecimal should parse");
    let encoded = serde_json::to_string(&secret).expect("secret should serialize");
    let decoded: Secret = serde_json::from_str(&encoded).expect("secret should deserialize");

    assert_eq!(encoded, "\"00abff\"");
    assert_eq!(decoded, secret);
    assert!(serde_json::from_str::<Secret>("\"not-hex\"").is_err());
}

#[test]
fn default_is_an_empty_secret() {
    let secret = Secret::default();

    assert!(secret.is_empty());
    assert!(secret.as_bytes().is_empty());
    assert_eq!(secret.to_hex_string(), "");
    assert!(secret.equal(&Secret::default()));
}

#[test]
fn equality_compares_secret_bytes() {
    let first = Secret::from_hex_string("abcd").expect("valid hexadecimal should parse");
    let same = Secret::from_hex_string("ABCD").expect("valid hexadecimal should parse");
    let different = Secret::from_hex_string("abce").expect("valid hexadecimal should parse");

    assert!(first.equal(&same));
    assert!(!first.equal(&different));
}

#[test]
fn new_returns_the_requested_number_of_random_bytes() {
    assert!(
        new(0)
            .expect("empty random secret should be supported")
            .is_empty()
    );

    let secret = new(32).expect("random secret generation should succeed");

    assert_eq!(secret.as_bytes().len(), 32);
}

#[test]
fn new_id_returns_128_bits_as_lowercase_hexadecimal() {
    let id = new_id().expect("ID generation should succeed");

    assert_eq!(id.len(), 32);
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
}

#[test]
fn random_alphanumeric_honors_length_and_character_set() {
    assert_eq!(
        random_alphanumeric(0).expect("zero length should succeed"),
        ""
    );

    for length in [1, 16, 255] {
        let value = random_alphanumeric(length).expect("random generation should succeed");
        assert_eq!(value.len(), length);
        assert!(
            value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        );
    }
}
