//! Secret generation and hexadecimal text encoding.
//!
//! This crate ports Go's `internal/secret` package. The source modules retain
//! the upstream filenames so future parity reviews can compare them directly.

mod id;
mod secret;

pub use id::{new_id, random_alphanumeric};
pub use secret::{Secret, SecretError, new};
