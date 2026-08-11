use oci_spec::distribution::Reference;

use crate::{FAINT, Style};

/// Formats an image reference in Docker's familiar form.
///
/// A tagged reference styles its name and tag independently and renders the
/// separating colon faintly. Invalid input is deliberately returned unchanged
/// except for the requested outer style.
#[must_use]
pub fn format_image(image: &str, style: Style) -> String {
    let Some(parsed) = parse_reference(image) else {
        return style.render(image);
    };

    let name = familiar_name(&parsed.reference, parsed.original_registry.as_deref());
    if let Some(digest) = parsed.reference.digest() {
        return style.render(format!("{name}@{digest}"));
    }

    match parsed.reference.tag() {
        Some(tag) => format!(
            "{}{}{}",
            style.render(name),
            FAINT.render(":"),
            style.render(tag)
        ),
        None => style.render(name),
    }
}

struct ParsedReference {
    reference: Reference,
    original_registry: Option<String>,
}

fn parse_reference(input: &str) -> Option<ParsedReference> {
    if input.len() == 64
        && input
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let (parse_input, original_registry) = normalize_special_registry(input)?;
    let reference: Reference = parse_input.parse().ok()?;
    if !is_ascii_tag(reference.tag()) || !is_lowercase_digest(reference.digest()) {
        return None;
    }

    Some(ParsedReference {
        reference,
        original_registry,
    })
}

fn normalize_special_registry(input: &str) -> Option<(String, Option<String>)> {
    if input.starts_with('[') {
        return normalize_ipv6_registry(input);
    }
    if let Some((first, remainder)) = input.split_once('/')
        && first.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Some((
            format!("uppercase.invalid/{remainder}"),
            Some(first.to_owned()),
        ));
    }
    Some((input.to_owned(), None))
}

fn normalize_ipv6_registry(input: &str) -> Option<(String, Option<String>)> {
    if !input.starts_with('[') {
        return Some((input.to_owned(), None));
    }

    let close = input.find(']')?;
    let host = &input[1..close];
    if host.is_empty() || host.contains('.') || host.parse::<std::net::Ipv6Addr>().is_err() {
        return None;
    }
    let after = &input[close + 1..];
    let slash = after.find('/')?;
    let port = &after[..slash];
    if !port.is_empty()
        && (!port.starts_with(':')
            || port.len() == 1
            || !port[1..].bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }

    let registry = format!("[{}]{}", host, port);
    let neutral = format!("ipv6.invalid{}{}", port, &after[slash..]);
    Some((neutral, Some(registry)))
}

fn is_ascii_tag(tag: Option<&str>) -> bool {
    tag.is_none_or(|tag| {
        let mut bytes = tag.bytes();
        bytes.next().is_some_and(is_tag_start)
            && bytes.all(|byte| is_tag_start(byte) || byte == b'.' || byte == b'-')
            && tag.len() <= 128
    })
}

const fn is_tag_start(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_lowercase_digest(digest: Option<&str>) -> bool {
    digest.is_none_or(|digest| {
        digest.split_once(':').is_some_and(|(_, encoded)| {
            encoded
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    })
}

fn familiar_name(reference: &Reference, original_registry: Option<&str>) -> String {
    let registry = original_registry.unwrap_or_else(|| reference.registry());
    let repository = reference.repository();
    if registry == "docker.io" {
        if let Some(official) = repository.strip_prefix("library/")
            && !official.contains('/')
        {
            return official.to_owned();
        }
        repository.to_owned()
    } else if original_registry.is_some()
        && matches!(reference.registry(), "ipv6.invalid" | "uppercase.invalid")
    {
        format!("{registry}/{repository}")
    } else if original_registry.is_some() {
        let repository = repository
            .strip_prefix(&format!("{registry}/"))
            .unwrap_or(repository);
        format!("{registry}/{repository}")
    } else {
        format!("{registry}/{repository}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GREEN, NO_STYLE};

    #[test]
    fn matches_familiar_docker_reference_cases() {
        let sha = "f".repeat(64);
        let cases = [
            ("ubuntu", "ubuntu:latest"),
            ("docker.io/library/ubuntu", "ubuntu:latest"),
            ("index.docker.io/library/ubuntu:24.04", "ubuntu:24.04"),
            ("docker.io/acme/widget", "acme/widget:latest"),
            (
                "localhost:5000/acme/widget:v2",
                "localhost:5000/acme/widget:v2",
            ),
            ("Foo/bar", "Foo/bar:latest"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                strip_sgr(&format_image(input, NO_STYLE)),
                expected,
                "{input}"
            );
        }

        let with_digest = format!("ubuntu:tag@sha256:{sha}");
        assert_eq!(
            format_image(&with_digest, NO_STYLE),
            format!("ubuntu@sha256:{sha}")
        );
    }

    #[test]
    fn supports_bracketed_ipv6_registry_compatibility() {
        assert_eq!(
            strip_sgr(&format_image("[fc00::1]:5000/repo:v1", NO_STYLE)),
            "[fc00::1]:5000/repo:v1"
        );
    }

    #[test]
    fn invalid_references_fall_back_to_original_text() {
        let invalid = [
            "",
            "repo:☃",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "repo@sha256:FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
            "[fc00::1/repo:v1",
            "[2001:db8:3:4::192.0.2.33]:5000/repo",
        ];
        for input in invalid {
            assert_eq!(format_image(input, NO_STYLE), input);
        }
    }

    #[test]
    fn tagged_parts_are_styled_separately() {
        let output = format_image("ubuntu:24.04", GREEN);
        assert_eq!(strip_sgr(&output), "ubuntu:24.04");
        assert!(output.matches("\x1b[").count() >= 6);
    }

    fn strip_sgr(value: &str) -> String {
        let mut out = String::new();
        let mut bytes = value.bytes().peekable();
        while let Some(byte) = bytes.next() {
            if byte == 0x1b && bytes.peek() == Some(&b'[') {
                bytes.next();
                for next in bytes.by_ref() {
                    if (b'@'..=b'~').contains(&next) {
                        break;
                    }
                }
            } else {
                out.push(char::from(byte));
            }
        }
        out
    }
}
