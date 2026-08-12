use std::fmt;

use ployz_internal_secret::{SecretError, new_id, random_alphanumeric};

#[derive(Debug)]
pub struct MachineNameError(SecretError);

impl fmt::Display for MachineNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "generate random suffix: {}", self.0)
    }
}

impl std::error::Error for MachineNameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

pub fn new_machine_id() -> Result<String, SecretError> {
    new_id()
}

pub fn new_random_machine_name() -> Result<String, MachineNameError> {
    random_alphanumeric(4)
        .map(|suffix| format!("machine-{suffix}"))
        .map_err(MachineNameError)
}

pub fn default_machine_name(
    hostname: &str,
    existing: &[impl AsRef<str>],
) -> Result<String, MachineNameError> {
    let mut name = machine_name_from_hostname(hostname);
    if name.is_empty() {
        name = new_random_machine_name()?;
    }
    if !contains(existing, &name) {
        return Ok(name);
    }
    for suffix in 1_u64.. {
        let candidate = format!("{name}-{suffix}");
        if !contains(existing, &candidate) {
            return Ok(candidate);
        }
    }
    unreachable!("the finite existing-name slice cannot contain every numeric suffix")
}

#[must_use]
pub fn machine_name_from_hostname(hostname: &str) -> String {
    let first_label = hostname
        .split_once('.')
        .map_or(hostname, |(label, _)| label);
    let sanitized: String = first_label
        .trim()
        .chars()
        .map(|character| {
            // Go applies one-code-point Unicode case mappings. Rust exposes
            // full expansions, so keep the simple mapping's first code point.
            let character = character
                .to_lowercase()
                .next()
                .expect("lowercase mappings are nonempty");
            if character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
            {
                character
            } else {
                '-'
            }
        })
        .collect();
    sanitized.trim_matches('-').to_owned()
}

fn contains(existing: &[impl AsRef<str>], candidate: &str) -> bool {
    existing.iter().any(|name| name.as_ref() == candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_cases_match_the_oracle_table() {
        let cases = [
            ("web", "web"),
            ("web-1.example.com", "web-1"),
            ("Web-Server", "web-server"),
            ("host_name@1", "host_name-1"),
            ("-_host_-", "_host_"),
            ("  myhost  ", "myhost"),
            ("", ""),
            ("@#", ""),
            (".example.com", ""),
            ("İX", "ix"),
        ];
        for (hostname, expected) in cases {
            assert_eq!(
                machine_name_from_hostname(hostname),
                expected,
                "{hostname:?}"
            );
        }
    }

    #[test]
    fn default_name_deduplicates_with_first_available_suffix() {
        assert_eq!(default_machine_name("web", &[] as &[&str]).unwrap(), "web");
        assert_eq!(default_machine_name("web", &["web"]).unwrap(), "web-1");
        assert_eq!(
            default_machine_name("web", &["web", "web-1", "web-3"]).unwrap(),
            "web-2"
        );
        assert_eq!(
            default_machine_name("My_Host", &[] as &[&str]).unwrap(),
            "my_host"
        );
    }

    #[test]
    fn invalid_hostname_uses_oracle_random_shape() {
        let name = default_machine_name("***", &[] as &[&str]).unwrap();
        let suffix = name.strip_prefix("machine-").expect("fixed prefix");
        assert_eq!(suffix.len(), 4);
        assert!(
            suffix
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        );
    }
}
