use crate::{SecretError, new};

const ALPHANUMERIC: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// Generates a unique random 128-bit ID as a lowercase hexadecimal string.
pub fn new_id() -> Result<String, SecretError> {
    new(16).map(|secret| secret.to_hex_string())
}

/// Generates a random string containing only `[a-z0-9]`.
pub fn random_alphanumeric(length: usize) -> Result<String, SecretError> {
    random_alphanumeric_with(length, getrandom::fill).map_err(SecretError::RandomNumber)
}

fn random_alphanumeric_with<E>(
    length: usize,
    mut fill: impl FnMut(&mut [u8]) -> Result<(), E>,
) -> Result<String, E> {
    let mut value = String::with_capacity(length);

    while value.len() < length {
        let mut random = [0_u8; 1];
        fill(&mut random)?;

        // Go's crypto/rand.Int masks each byte to the maximum's bit length,
        // then retries values outside the requested range.
        let candidate = random[0] & 0x3f;
        if usize::from(candidate) < ALPHANUMERIC.len() {
            value.push(ALPHANUMERIC[usize::from(candidate)] as char);
        }
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampler_matches_crypto_rand_int_masking_and_rejection() {
        let mut bytes = [0x00, 0x23, 0x24, 0xff, 0x40].into_iter();
        let generated = random_alphanumeric_with(3, |output| {
            output[0] = bytes.next().expect("fixture has enough entropy bytes");
            Ok::<_, std::convert::Infallible>(())
        })
        .expect("infallible entropy fixture");

        assert_eq!(generated, "a9a");
    }

    #[test]
    fn sampler_propagates_entropy_failure_without_partial_output() {
        let error = random_alphanumeric_with(4, |_| Err("entropy unavailable"))
            .expect_err("entropy failure must propagate");

        assert_eq!(error, "entropy unavailable");
    }
}
