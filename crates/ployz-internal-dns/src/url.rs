use oxiri::{Iri, IriRef};

use crate::Error;

#[derive(Clone, Debug)]
pub(crate) struct CompatibleUrl {
    full: String,
}

impl CompatibleUrl {
    pub(crate) fn parse(raw: &str) -> Result<Self, Error> {
        let escaped = escape_for_go(raw)?;
        let checked_end = escaped
            .find('?')
            .unwrap_or(escaped.len())
            .min(escaped.find('#').unwrap_or(escaped.len()));
        Iri::parse(&escaped[..checked_end])
            .map_err(|e| Error::InvalidUrl(format!("parse {raw:?}: {e}")))?;
        Ok(Self { full: escaped })
    }

    pub(crate) fn append(endpoint: &str, suffix: &str) -> Result<Self, Error> {
        Self::parse(&format!("{endpoint}/{suffix}"))
    }

    pub(crate) fn request_uri(&self) -> Result<String, Error> {
        let no_fragment = self
            .full
            .split_once('#')
            .map_or(self.full.as_str(), |(head, _)| head);
        let scheme = no_fragment.find("://").ok_or_else(|| {
            Error::InvalidUrl(format!("unsupported protocol scheme in {no_fragment:?}"))
        })?;
        let authority_start = scheme + 3;
        let authority_end = no_fragment[authority_start..]
            .find(['/', '?'])
            .map_or(no_fragment.len(), |i| authority_start + i);
        let authority = &no_fragment[authority_start..authority_end];
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if host.is_empty() {
            return Err(Error::InvalidUrl(format!(
                "missing host in {:?}",
                self.full
            )));
        }
        let clean = format!(
            "{}{}{}",
            &no_fragment[..authority_start],
            host,
            &no_fragment[authority_end..]
        );
        clean
            .parse::<http::Uri>()
            .map_err(|e| Error::InvalidUrl(format!("parse {:?}: {e}", self.full)))?;
        Ok(clean)
    }

    pub(crate) fn resolve(&self, reference: &str) -> Result<Self, Error> {
        let escaped_ref = escape_reference(reference)?;
        let (base_core, base_query, base_fragment) = components(&self.full);
        let (reference_core, reference_query, reference_fragment) = components(&escaped_ref);

        let resolved = if reference_core.is_empty() {
            let core = normalize_base_core(base_core)?;
            let query = reference_query.or(base_query);
            let fragment = match (reference_query, reference_fragment) {
                (_, Some(fragment)) => Some(fragment),
                (Some(_), None) => None,
                (None, None) => base_fragment,
            };
            assemble(&core, query, fragment)
        } else {
            let base =
                Iri::parse(base_core).map_err(|error| Error::InvalidRedirect(error.to_string()))?;
            let reference = IriRef::parse(reference_core)
                .map_err(|error| Error::InvalidRedirect(error.to_string()))?;
            let core = base
                .resolve(&reference)
                .map_err(|error| Error::InvalidRedirect(error.to_string()))?;
            assemble(core.as_str(), reference_query, reference_fragment)
        };
        Self::parse(&resolved).map_err(|error| Error::InvalidRedirect(error.to_string()))
    }

    pub(crate) fn host(&self) -> &str {
        let start = self.full.find("://").map_or(0, |i| i + 3);
        let end = self.full[start..]
            .find(['/', '?', '#'])
            .map_or(self.full.len(), |i| start + i);
        let authority = &self.full[start..end];
        let hostport = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if hostport.starts_with('[') {
            return hostport
                .split_once(']')
                .map_or(hostport, |(host, _)| &host[1..]);
        }
        hostport
            .rsplit_once(':')
            .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
            .map_or(hostport, |(host, _)| host)
    }

    pub(crate) fn scheme(&self) -> &str {
        self.full.split_once(':').map_or("", |(scheme, _)| scheme)
    }

    pub(crate) fn userinfo(&self) -> Option<(String, String)> {
        let start = self.full.find("://")? + 3;
        let end = self.full[start..]
            .find(['/', '?', '#'])
            .map_or(self.full.len(), |i| start + i);
        let (userinfo, _) = self.full[start..end].rsplit_once('@')?;
        let (username, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
        Some((percent_decode(username), percent_decode(password)))
    }

    pub(crate) fn without_userinfo(&self) -> String {
        let Some(scheme) = self.full.find("://") else {
            return self.full.clone();
        };
        let start = scheme + 3;
        let end = self.full[start..]
            .find(['/', '?', '#'])
            .map_or(self.full.len(), |i| start + i);
        let auth = &self.full[start..end];
        auth.rsplit_once('@').map_or_else(
            || self.full.clone(),
            |(_, host)| format!("{}{}{}", &self.full[..start], host, &self.full[end..]),
        )
    }

    pub(crate) fn referer(&self) -> String {
        self.without_userinfo()
            .split_once('#')
            .map_or_else(|| self.without_userinfo(), |(head, _)| head.to_owned())
    }
}

fn escape_for_go(raw: &str) -> Result<String, Error> {
    let Some(colon) = raw.find(':') else {
        return Err(Error::InvalidUrl(format!(
            "parse {raw:?}: missing protocol scheme"
        )));
    };
    let scheme = &raw[..colon];
    if scheme.is_empty()
        || !scheme.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_alphabetic()
                || (i > 0 && (b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.')))
        })
    {
        return Err(Error::InvalidUrl(format!(
            "parse {raw:?}: invalid URL scheme"
        )));
    }
    let mut out = scheme.to_ascii_lowercase();
    out.push_str(&raw[colon..]);
    let (core, query, fragment) = components(&out);
    validate_percent(core, raw)?;
    if let Some(fragment) = fragment {
        validate_percent(fragment, raw)?;
    }
    validate_host(core, raw)?;
    let path_start = core
        .find("://")
        .and_then(|i| core[i + 3..].find('/').map(|p| i + 3 + p))
        .unwrap_or(core.len());
    let mut result = format!(
        "{}{}",
        &core[..path_start],
        percent_escape(&core[path_start..])
    );
    if let Some(query) = query {
        result.push('?');
        result.push_str(query);
    }
    if let Some(fragment) = fragment {
        result.push('#');
        result.push_str(&percent_escape(fragment));
    }
    if let Some(scheme_end) = result.find("://") {
        let start = scheme_end + 3;
        let end = result[start..]
            .find(['/', '?', '#'])
            .map_or(result.len(), |i| start + i);
        if result[start..end].ends_with(':') {
            result.remove(end - 1);
        }
    }
    Ok(result)
}

fn escape_reference(raw: &str) -> Result<String, Error> {
    let (core, query, fragment) = components(raw);
    validate_percent(core, raw)?;
    if let Some(fragment) = fragment {
        validate_percent(fragment, raw)?;
    }
    if core.contains("://") {
        validate_host(core, raw)?;
    }
    Ok(assemble(
        &percent_escape(core),
        query,
        fragment.map(percent_escape).as_deref(),
    ))
}

fn components(value: &str) -> (&str, Option<&str>, Option<&str>) {
    let (before_fragment, fragment) = value
        .split_once('#')
        .map_or((value, None), |(head, tail)| (head, Some(tail)));
    let (core, query) = before_fragment
        .split_once('?')
        .map_or((before_fragment, None), |(head, tail)| (head, Some(tail)));
    (core, query, fragment)
}

fn assemble(core: &str, query: Option<&str>, fragment: Option<&str>) -> String {
    let mut value = core.to_owned();
    if let Some(query) = query {
        value.push('?');
        value.push_str(query);
    }
    if let Some(fragment) = fragment {
        value.push('#');
        value.push_str(fragment);
    }
    value
}

fn normalize_base_core(core: &str) -> Result<String, Error> {
    let Some(scheme) = core.find("://") else {
        return Ok(core.to_owned());
    };
    let authority_start = scheme + 3;
    let authority_end = core[authority_start..]
        .find('/')
        .map_or(core.len(), |index| authority_start + index);
    if authority_end == core.len() {
        return Ok(core.to_owned());
    }
    let origin = format!("{}/", &core[..authority_end]);
    let base =
        Iri::parse(origin.as_str()).map_err(|error| Error::InvalidRedirect(error.to_string()))?;
    let path = IriRef::parse(&core[authority_end..])
        .map_err(|error| Error::InvalidRedirect(error.to_string()))?;
    base.resolve(&path)
        .map(|resolved| resolved.to_string())
        .map_err(|error| Error::InvalidRedirect(error.to_string()))
}

fn validate_host(core: &str, raw: &str) -> Result<(), Error> {
    let Some(scheme) = core.find("://") else {
        return Ok(());
    };
    let start = scheme + 3;
    let end = core[start..]
        .find('/')
        .map_or(core.len(), |index| start + index);
    let authority = &core[start..end];
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if !host.starts_with('[') && host.contains('%') {
        return Err(Error::InvalidUrl(format!(
            "parse {raw:?}: invalid URL escape in host"
        )));
    }
    Ok(())
}

fn validate_percent(value: &str, raw: &str) -> Result<(), Error> {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && (i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit())
        {
            return Err(Error::InvalidUrl(format!(
                "parse {raw:?}: invalid URL escape"
            )));
        }
        i += if bytes[i] == b'%' { 3 } else { 1 };
    }
    Ok(())
}

fn percent_escape(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte == b' ' || byte == b'\\' || !byte.is_ascii() {
            out.push_str(&format!("%{byte:02X}"));
        } else {
            out.push(byte as char);
        }
    }
    out
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => 0,
            };
            out.push((hex(bytes[i + 1]) << 4) | hex(bytes[i + 2]));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::CompatibleUrl;

    #[test]
    fn construction_preserves_go_observable_spelling() {
        assert_eq!(
            CompatibleUrl::append("HtTp://EXAMPLE.test:/a/../b/./c", "domains")
                .unwrap()
                .request_uri()
                .unwrap(),
            "http://EXAMPLE.test/a/../b/./c/domains"
        );
        assert_eq!(
            CompatibleUrl::append("https://example.test/v 1", "domains")
                .unwrap()
                .request_uri()
                .unwrap(),
            "https://example.test/v%201/domains"
        );
        assert_eq!(
            CompatibleUrl::append("https://example.test/a\\b", "domains")
                .unwrap()
                .request_uri()
                .unwrap(),
            "https://example.test/a%5Cb/domains"
        );
        assert!(CompatibleUrl::append("https://example.test/bad%zz", "domains").is_err());
        assert_eq!(
            CompatibleUrl::append("https://h/p?q=%zz", "domains")
                .unwrap()
                .request_uri()
                .unwrap(),
            "https://h/p?q=%zz/domains"
        );
        assert!(CompatibleUrl::parse("https://h/path#bad%zz").is_err());
        assert!(CompatibleUrl::parse("https://%65xample.test/path").is_err());
    }

    #[test]
    fn oxiri_resolution_preserves_encoded_dot_segments_and_repeated_slashes() {
        let base =
            CompatibleUrl::parse("https://user:pass@EXAMPLE.test:443/a/b/c?old=1#old").unwrap();
        assert_eq!(
            base.resolve("/a//b/../c").unwrap().without_userinfo(),
            "https://EXAMPLE.test:443/a//c"
        );
        assert_eq!(
            base.resolve("%2e%2e/x").unwrap().without_userinfo(),
            "https://EXAMPLE.test:443/a/b/%2e%2e/x"
        );
        assert_eq!(
            base.resolve("..\\x").unwrap().without_userinfo(),
            "https://EXAMPLE.test:443/a/b/..%5Cx"
        );
    }

    #[test]
    fn redirect_seam_carries_raw_query_and_normalizes_dotty_base_paths() {
        let base = CompatibleUrl::parse("https://EXAMPLE.test:443/a/../b/./c?q=%zz#old").unwrap();
        assert_eq!(
            base.resolve("").unwrap().without_userinfo(),
            "https://EXAMPLE.test:443/b/c?q=%zz#old"
        );
        assert_eq!(
            base.resolve("?next=%zz").unwrap().without_userinfo(),
            "https://EXAMPLE.test:443/b/c?next=%zz"
        );
        assert_eq!(
            base.resolve("#new").unwrap().without_userinfo(),
            "https://EXAMPLE.test:443/b/c?q=%zz#new"
        );
    }
}
