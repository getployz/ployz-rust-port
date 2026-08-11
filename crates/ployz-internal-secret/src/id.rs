use crate::{SecretError, new};

const ALPHANUMERIC: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// Generates a unique random 128-bit ID as a lowercase hexadecimal string.
pub fn new_id() -> Result<String, SecretError> {
    new(16).map(|secret| secret.to_hex_string())
}

/// Generates a random string containing only `[a-z0-9]`.
pub fn random_alphanumeric(length: usize) -> Result<String, SecretError> {
    let mut value = String::with_capacity(length);

    while value.len() < length {
        let mut random = [0_u8; 1];
        getrandom::fill(&mut random).map_err(SecretError::RandomNumber)?;

        // Reject the incomplete upper range so modulo selection remains uniform.
        if random[0] < 252 {
            value.push(ALPHANUMERIC[usize::from(random[0]) % ALPHANUMERIC.len()] as char);
        }
    }

    Ok(value)
}
