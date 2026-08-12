use std::net::Ipv6Addr;

use oci_spec::distribution::Reference;

use crate::DockerError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImageReference {
    pub(crate) name: String,
    pub(crate) tag: Option<String>,
    pub(crate) digest: Option<String>,
}

impl ImageReference {
    pub(crate) fn parse(input: &str) -> Result<Self, DockerError> {
        if input.len() == 64
            && input
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid_reference(input));
        }
        let (parsed, registry_override) =
            if let Some((registry, remainder)) = compat_registry(input)? {
                let substituted = format!("compat.invalid/{remainder}");
                (parse_reference(input, &substituted)?, Some(registry))
            } else {
                (parse_reference(input, input)?, None)
            };
        validate_tag(parsed.tag(), input)?;
        validate_digest(parsed.digest(), input)?;
        let registry = registry_override
            .as_deref()
            .unwrap_or_else(|| parsed.registry());
        Ok(Self {
            name: format!("{registry}/{}", parsed.repository()),
            tag: parsed.tag().map(str::to_owned),
            digest: parsed.digest().map(str::to_owned),
        })
    }

    pub(crate) fn api_tag(&self) -> Option<&str> {
        self.digest.as_deref().or(self.tag.as_deref())
    }
}

fn parse_reference(original: &str, parseable: &str) -> Result<Reference, DockerError> {
    parseable.parse::<Reference>().map_err(|error| {
        DockerError::Configuration(format!("invalid image reference {original:?}: {error}"))
    })
}

fn compat_registry(input: &str) -> Result<Option<(String, &str)>, DockerError> {
    if input.starts_with('[') {
        let close = input.find(']').ok_or_else(|| invalid_reference(input))?;
        input[1..close]
            .parse::<Ipv6Addr>()
            .map_err(|_| invalid_reference(input))?;
        let slash = input[close + 1..]
            .find('/')
            .map(|offset| close + 1 + offset)
            .ok_or_else(|| invalid_reference(input))?;
        let suffix = &input[close + 1..slash];
        if !suffix.is_empty()
            && (!suffix.starts_with(':')
                || suffix.len() == 1
                || !suffix[1..].bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(invalid_reference(input));
        }
        let remainder = &input[slash + 1..];
        if remainder.is_empty() {
            return Err(invalid_reference(input));
        }
        return Ok(Some((input[..slash].to_owned(), remainder)));
    }

    if let Some((first, remainder)) = input.split_once('/')
        && first.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        if remainder.is_empty() || !valid_domain_and_port(first) {
            return Err(invalid_reference(input));
        }
        return Ok(Some((first.to_owned(), remainder)));
    }
    Ok(None)
}

fn valid_domain_and_port(value: &str) -> bool {
    let (domain, port) = match value.rsplit_once(':') {
        Some((domain, port)) => (domain, Some(port)),
        None => (value, None),
    };
    if domain.is_empty()
        || port.is_some_and(|port| port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    domain.split('.').all(|component| {
        let bytes = component.as_bytes();
        !bytes.is_empty()
            && bytes[0].is_ascii_alphanumeric()
            && bytes[bytes.len() - 1].is_ascii_alphanumeric()
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
    })
}

fn validate_tag(tag: Option<&str>, input: &str) -> Result<(), DockerError> {
    if tag.is_some_and(|tag| !tag.is_ascii()) {
        return Err(invalid_reference(input));
    }
    Ok(())
}

fn validate_digest(digest: Option<&str>, input: &str) -> Result<(), DockerError> {
    let Some(digest) = digest else {
        return Ok(());
    };
    let Some((algorithm, encoded)) = digest.split_once(':') else {
        return Err(invalid_reference(input));
    };
    let expected = match algorithm {
        "sha256" => 64,
        "sha384" => 96,
        "sha512" => 128,
        _ => return Err(invalid_reference(input)),
    };
    if encoded.len() != expected
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid_reference(input));
    }
    Ok(())
}

fn invalid_reference(input: &str) -> DockerError {
    DockerError::Configuration(format!("invalid image reference: {input}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_docker_names_tags_and_digests() {
        let simple = ImageReference::parse("busybox").unwrap();
        assert_eq!(simple.name, "docker.io/library/busybox");
        assert_eq!(simple.tag.as_deref(), Some("latest"));

        let tagged = ImageReference::parse("registry.example:5000/ns/app:Tag").unwrap();
        assert_eq!(tagged.name, "registry.example:5000/ns/app");
        assert_eq!(tagged.tag.as_deref(), Some("Tag"));

        let digest = format!("busybox@sha256:{}", "f".repeat(64));
        let digested = ImageReference::parse(&digest).unwrap();
        assert_eq!(
            digested.api_tag(),
            digest.split_once('@').map(|(_, value)| value)
        );
    }

    #[test]
    fn preserves_approved_compatibility_registries() {
        let ipv6 = ImageReference::parse("[fc00::1]:5000/repo:tag").unwrap();
        assert_eq!(ipv6.name, "[fc00::1]:5000/repo");
        let uppercase = ImageReference::parse("Foo/repo:tag").unwrap();
        assert_eq!(uppercase.name, "Foo/repo");
        let uppercase_port = ImageReference::parse("Foo.EXAMPLE:5000/repo:tag").unwrap();
        assert_eq!(uppercase_port.name, "Foo.EXAMPLE:5000/repo");
    }

    #[test]
    fn rejects_url_delimiters_unicode_tags_and_uppercase_digest_hex() {
        assert!(ImageReference::parse("repo?query:tag").is_err());
        assert!(ImageReference::parse("repo#fragment:tag").is_err());
        assert!(ImageReference::parse("Foo?/repo").is_err());
        assert!(ImageReference::parse("Foo#/repo").is_err());
        assert!(ImageReference::parse("Foo_:5000/repo").is_err());
        assert!(ImageReference::parse("repo:täg").is_err());
        let digest = format!("repo@sha256:{}A", "f".repeat(63));
        assert!(ImageReference::parse(&digest).is_err());
        assert!(ImageReference::parse(&"f".repeat(64)).is_err());
        assert!(ImageReference::parse(&"0".repeat(64)).is_err());
        assert!(ImageReference::parse(&format!("repo@sha256:{}", "f".repeat(64))).is_ok());
    }
}
