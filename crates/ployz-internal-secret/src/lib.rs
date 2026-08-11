//! Secret generation and hexadecimal text encoding.
//!
//! Secrets preserve the absent zero value used by configuration callers while
//! formatting and comparing it like an empty byte sequence.

mod id;
mod secret;

pub use id::{new_id, random_alphanumeric};
pub use secret::{Secret, SecretError, new};
