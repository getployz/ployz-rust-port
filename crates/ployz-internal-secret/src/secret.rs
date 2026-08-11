use std::{error::Error, fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// An opaque sequence of secret bytes.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct Secret(Vec<u8>);

/// An error encountered while parsing or generating a secret.
#[derive(Debug)]
pub enum SecretError {
    InvalidHex(hex::FromHexError),
    RandomBytes(getrandom::Error),
    RandomNumber(getrandom::Error),
}

impl Secret {
    /// Parses a hexadecimal string into a secret.
    pub fn from_hex_string(value: &str) -> Result<Self, SecretError> {
        hex::decode(value)
            .map(Self)
            .map_err(SecretError::InvalidHex)
    }

    /// Returns the lowercase hexadecimal representation of this secret.
    #[must_use]
    pub fn to_hex_string(&self) -> String {
        hex::encode(&self.0)
    }

    /// Returns the bytes contained in this secret.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Reports whether this secret contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Compares this secret with another secret.
    #[must_use]
    pub fn equal(&self, other: &Self) -> bool {
        self == other
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
            Self::InvalidHex(source) => write!(formatter, "invalid hex-encoded secret: {source}"),
            Self::RandomBytes(source) => source.fmt(formatter),
            Self::RandomNumber(source) => write!(formatter, "get random number: {source}"),
        }
    }
}

impl Error for SecretError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidHex(source) => Some(source),
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
        let value = String::deserialize(deserializer)?;
        Self::from_hex_string(&value).map_err(D::Error::custom)
    }
}

/// Generates a cryptographically random secret with `length` bytes.
pub fn new(length: usize) -> Result<Secret, SecretError> {
    let mut bytes = vec![0; length];
    getrandom::fill(&mut bytes).map_err(SecretError::RandomBytes)?;
    Ok(Secret(bytes))
}
