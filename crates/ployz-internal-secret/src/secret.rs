use std::{
    error::Error,
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// A sequence of secret bytes.
///
/// The absent state preserves the distinction between a Go nil slice and an
/// allocated empty slice. Both states have the same bytes, text, and equality
/// semantics.
#[derive(Clone, Default)]
pub struct Secret(Option<Vec<u8>>);

/// An error encountered while parsing or generating a secret.
#[derive(Debug)]
pub enum SecretError {
    InvalidHex(hex::FromHexError),
    RandomBytes(getrandom::Error),
    RandomNumber(getrandom::Error),
}

impl Secret {
    /// Parses a hexadecimal string into a present secret.
    pub fn from_hex_string(value: &str) -> Result<Self, SecretError> {
        decode_hex(value).map(|bytes| Self(Some(bytes)))
    }

    /// Returns the lowercase hexadecimal representation of this secret.
    #[must_use]
    pub fn to_hex_string(&self) -> String {
        hex::encode(self.as_bytes())
    }

    /// Returns the bytes contained in this secret.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_deref().unwrap_or_default()
    }

    /// Returns mutable access to the bytes contained in this secret.
    pub fn as_mut_bytes(&mut self) -> &mut [u8] {
        self.0.as_deref_mut().unwrap_or_default()
    }

    /// Consumes the secret and returns its bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0.unwrap_or_default()
    }

    /// Reports whether this is the absent zero value.
    #[must_use]
    pub const fn is_nil(&self) -> bool {
        self.0.is_none()
    }

    /// Reports whether this secret contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    /// Compares this secret with another secret.
    #[must_use]
    pub fn equal(&self, other: &Self) -> bool {
        self == other
    }
}

impl AsRef<[u8]> for Secret {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl AsMut<[u8]> for Secret {
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_mut_bytes()
    }
}

impl From<Vec<u8>> for Secret {
    fn from(bytes: Vec<u8>) -> Self {
        Self(Some(bytes))
    }
}

impl From<&[u8]> for Secret {
    fn from(bytes: &[u8]) -> Self {
        Self::from(bytes.to_vec())
    }
}

impl<const LENGTH: usize> From<[u8; LENGTH]> for Secret {
    fn from(bytes: [u8; LENGTH]) -> Self {
        Self::from(Vec::from(bytes))
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Secret {}

impl Hash for Secret {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Secret")
            .field(&self.as_bytes())
            .finish()
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex_string())
    }
}

impl fmt::Display for SecretError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHex(source) => {
                formatter.write_str("invalid hex-encoded secret: ")?;
                format_go_hex_error(*source, formatter)
            }
            Self::RandomBytes(source) => source.fmt(formatter),
            Self::RandomNumber(source) => write!(formatter, "get random number: {source}"),
        }
    }
}

impl Error for SecretError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidHex(source) => Some(source),
            // getrandom's workspace-selected feature set does not implement
            // std::error::Error; the public variants retain the typed source.
            Self::RandomBytes(_) | Self::RandomNumber(_) => None,
        }
    }
}

impl FromStr for Secret {
    type Err = SecretError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_hex_string(value)
    }
}

impl Serialize for Secret {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex_string())
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer)?
            .map_or_else(
                || Ok(Self::default()),
                |value| Self::from_hex_string(&value),
            )
            .map_err(D::Error::custom)
    }
}

/// Generates a cryptographically random present secret with `length` bytes.
pub fn new(length: usize) -> Result<Secret, SecretError> {
    let mut bytes = vec![0; length];
    getrandom::fill(&mut bytes).map_err(SecretError::RandomBytes)?;
    Ok(Secret::from(bytes))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, SecretError> {
    let input = value.as_bytes();
    let mut decoded = Vec::with_capacity(input.len() / 2);
    let mut pairs = input.chunks_exact(2);

    for (pair_index, pair) in pairs.by_ref().enumerate() {
        let index = pair_index * 2;
        let high = decode_nibble(pair[0]).ok_or_else(|| invalid_hex(pair[0], index))?;
        let low = decode_nibble(pair[1]).ok_or_else(|| invalid_hex(pair[1], index + 1))?;
        decoded.push((high << 4) | low);
    }

    if let [byte] = pairs.remainder() {
        if decode_nibble(*byte).is_none() {
            return Err(invalid_hex(*byte, input.len() - 1));
        }
        return Err(SecretError::InvalidHex(hex::FromHexError::OddLength));
    }

    Ok(decoded)
}

const fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn invalid_hex(byte: u8, index: usize) -> SecretError {
    SecretError::InvalidHex(hex::FromHexError::InvalidHexCharacter {
        c: char::from(byte),
        index,
    })
}

fn format_go_hex_error(
    source: hex::FromHexError,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match source {
        hex::FromHexError::OddLength => formatter.write_str("encoding/hex: odd length hex string"),
        hex::FromHexError::InvalidHexCharacter { c, .. } => {
            let byte = c as u32;
            write!(formatter, "encoding/hex: invalid byte: U+{byte:04X}")?;
            if let Some(quoted) = go_printable_byte(c) {
                write!(formatter, " {quoted}")?;
            }
            Ok(())
        }
        hex::FromHexError::InvalidStringLength => {
            formatter.write_str("encoding/hex: odd length hex string")
        }
    }
}

fn go_printable_byte(character: char) -> Option<String> {
    let byte = character as u32;
    let printable = matches!(byte, 0x20..=0x7e | 0xa1..=0xac | 0xae..=0xff);
    printable.then(|| format!("'{character}'"))
}
