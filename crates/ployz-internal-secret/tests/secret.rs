use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    str::FromStr,
};

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
    let invalid_odd_tail =
        Secret::from_hex_string("abz").expect_err("an invalid trailing digit must fail");
    let non_ascii = Secret::from_hex_string("é").expect_err("non-ASCII input must fail");
    let control = Secret::from_hex_string("\n").expect_err("control input must fail");
    let quote = Secret::from_hex_string("'").expect_err("quote input must fail");
    let backslash = Secret::from_hex_string("\\").expect_err("backslash input must fail");

    assert_eq!(
        odd_length.to_string(),
        "invalid hex-encoded secret: encoding/hex: odd length hex string"
    );
    assert_eq!(
        non_hex.to_string(),
        "invalid hex-encoded secret: encoding/hex: invalid byte: U+007A 'z'"
    );
    assert_eq!(
        invalid_odd_tail.to_string(),
        "invalid hex-encoded secret: encoding/hex: invalid byte: U+007A 'z'"
    );
    assert_eq!(
        non_ascii.to_string(),
        "invalid hex-encoded secret: encoding/hex: invalid byte: U+00C3 'Ã'"
    );
    assert_eq!(
        control.to_string(),
        "invalid hex-encoded secret: encoding/hex: invalid byte: U+000A"
    );
    assert_eq!(
        quote.to_string(),
        "invalid hex-encoded secret: encoding/hex: invalid byte: U+0027 '''"
    );
    assert_eq!(
        backslash.to_string(),
        "invalid hex-encoded secret: encoding/hex: invalid byte: U+005C '\\'"
    );
}

#[test]
fn hexadecimal_round_trips_arbitrary_bytes() {
    for length in 0..=512 {
        let bytes: Vec<u8> = (0..length)
            .map(|index| (index as u8).wrapping_mul(73).wrapping_add(19))
            .collect();
        let secret = Secret::from(bytes.as_slice());
        let encoded = secret.to_hex_string();

        assert_eq!(encoded.len(), length * 2);
        assert!(
            encoded
                .bytes()
                .all(|byte| { byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) })
        );
        assert_eq!(
            Secret::from_hex_string(&encoded)
                .expect("the crate's own hexadecimal output must parse")
                .as_bytes(),
            bytes
        );
    }
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
fn serde_null_restores_the_nil_zero_value() {
    let secret: Secret = serde_json::from_str("null").expect("JSON null should deserialize");

    assert!(secret.is_nil());
    assert!(secret.is_empty());
    assert_eq!(serde_json::to_string(&secret).unwrap(), "\"\"");
}

#[test]
fn default_is_an_empty_secret() {
    let secret = Secret::default();

    assert!(secret.is_nil());
    assert!(secret.is_empty());
    assert!(secret.as_bytes().is_empty());
    assert_eq!(secret.to_hex_string(), "");
    assert!(secret.equal(&Secret::default()));
}

#[test]
fn nil_and_allocated_empty_secrets_compare_and_hash_equally() {
    let nil = Secret::default();
    let parsed_empty = Secret::from_hex_string("").expect("empty hexadecimal should parse");
    let converted_empty = Secret::from(Vec::new());

    assert!(nil.is_nil());
    assert!(!parsed_empty.is_nil());
    assert!(!converted_empty.is_nil());
    assert_eq!(nil, parsed_empty);
    assert_eq!(nil, converted_empty);

    let hash = |secret: &Secret| {
        let mut hasher = DefaultHasher::new();
        secret.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(hash(&nil), hash(&parsed_empty));
    assert_eq!(hash(&nil), hash(&converted_empty));
}

#[test]
fn raw_byte_conversions_and_views_support_callers() {
    let mut from_slice = Secret::from(&[0xde, 0xad, 0xbe, 0xef][..]);
    let from_array = Secret::from([0xde, 0xad, 0xbe, 0xef]);

    assert_eq!(from_slice, from_array);
    assert_eq!(from_slice.as_ref(), &[0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(&from_slice.as_bytes()[..2], &[0xde, 0xad]);
    from_slice.as_mut()[0] = 0xca;
    assert_eq!(from_slice.as_mut_bytes(), &[0xca, 0xad, 0xbe, 0xef]);
    assert_eq!(from_array.into_bytes(), vec![0xde, 0xad, 0xbe, 0xef]);
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
    let empty = new(0).expect("empty random secret should be supported");
    assert!(empty.is_empty());
    assert!(!empty.is_nil());

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
