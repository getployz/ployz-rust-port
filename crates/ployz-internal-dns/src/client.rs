use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Deserializer, de};

use crate::{
    CreateRecordsError, DomainResponse, Error, Header, RecordRequest, RecordResponse, Request,
    Response, Transport, transport::DefaultTransport, url::CompatibleUrl,
};

struct AuthErrorResponse {
    data: AuthErrorData,
}
#[derive(Default)]
struct AuthErrorData {
    no_domain: bool,
}

impl<'de> Deserialize<'de> for AuthErrorResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = AuthErrorResponse;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a DNS authentication error")
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut data = AuthErrorData::default();
                while let Some(key) = map.next_key::<String>()? {
                    if key.eq_ignore_ascii_case("status") {
                        let _: Option<i64> = map.next_value()?;
                    } else if key.eq_ignore_ascii_case("msg") {
                        let _: Option<String> = map.next_value()?;
                    } else if key.eq_ignore_ascii_case("data") {
                        if let Some(patch) = map.next_value::<Option<AuthErrorPatch>>()?
                            && let Some(no_domain) = patch.no_domain
                        {
                            data.no_domain = no_domain;
                        }
                    } else {
                        map.next_value::<de::IgnoredAny>()?;
                    }
                }
                Ok(AuthErrorResponse { data })
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

#[derive(Default)]
struct AuthErrorPatch {
    no_domain: Option<bool>,
}

impl<'de> Deserialize<'de> for AuthErrorPatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = AuthErrorPatch;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("DNS authentication error data")
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut no_domain = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key.eq_ignore_ascii_case("noDomain") {
                        if let Some(value) = map.next_value::<Option<bool>>()? {
                            no_domain = Some(value);
                        }
                    } else {
                        map.next_value::<de::IgnoredAny>()?;
                    }
                }
                Ok(AuthErrorPatch { no_domain })
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

#[derive(Clone)]
pub struct Client {
    transport: Arc<dyn Transport>,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        Self {
            transport: Arc::new(DefaultTransport::shared()),
        }
    }
    pub fn with_transport(transport: impl Transport) -> Self {
        Self {
            transport: Arc::new(transport),
        }
    }

    pub fn reserve_domain(&self, endpoint: &str) -> Result<(String, String), Error> {
        let url = CompatibleUrl::append(endpoint, "domains")?;
        let response: DomainResponse = self.post(url, None, "")?;
        Ok((response.name, response.token))
    }

    pub fn create_records(
        &self,
        endpoint: &str,
        domain: &str,
        token: &str,
        records: &[RecordRequest],
    ) -> Result<Vec<RecordResponse>, CreateRecordsError> {
        if records.is_empty() {
            return Ok(Vec::new());
        }
        let url = CompatibleUrl::append(endpoint, &format!("domains/{domain}/records"))
            .map_err(|e| CreateRecordsError::new(Vec::new(), e))?;
        let mut completed = Vec::with_capacity(records.len());
        for record in records {
            let body = go_json_line(record);
            match self.post(url.clone(), Some(body), token) {
                Ok(response) => completed.push(response),
                Err(error) => return Err(CreateRecordsError::new(completed, error)),
            }
        }
        Ok(completed)
    }

    fn post<T: for<'de> Deserialize<'de>>(
        &self,
        url: CompatibleUrl,
        body: Option<Vec<u8>>,
        token: &str,
    ) -> Result<T, Error> {
        let mut headers = vec![Header {
            name: "Content-Type".into(),
            value: "application/json".into(),
        }];
        if !token.is_empty() {
            headers.push(Header {
                name: "Authorization".into(),
                value: format!("Bearer {token}"),
            });
        }
        let response = self.follow(url, "POST", body, headers)?;
        self.decode(response)
    }

    fn follow(
        &self,
        mut url: CompatibleUrl,
        mut method: &str,
        mut body: Option<Vec<u8>>,
        mut headers: Vec<Header>,
    ) -> Result<Response, Error> {
        let initial_host = url.host().to_ascii_lowercase();
        for request_number in 1..=10 {
            let request_url = url.request_uri()?;
            let mut request_headers = headers.clone();
            if !request_headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("authorization"))
                && let Some((username, password)) = url.userinfo()
            {
                request_headers.push(Header {
                    name: "Authorization".into(),
                    value: format!(
                        "Basic {}",
                        STANDARD.encode(format!("{username}:{password}"))
                    ),
                });
            }
            request_headers.push(Header {
                name: "User-Agent".into(),
                value: "Go-http-client/1.1".into(),
            });
            request_headers.push(Header {
                name: "Accept-Encoding".into(),
                value: "gzip".into(),
            });
            tracing::debug!(method, url = %url.without_userinfo(), "Making request to DNS service.");
            let response = self
                .transport
                .execute(Request {
                    method: method.into(),
                    url: request_url,
                    headers: request_headers,
                    body: body.clone(),
                })
                .map_err(|error| {
                    if error.is_response_body() {
                        Error::ReadResponse(std::io::Error::other(error))
                    } else {
                        Error::Transport(error)
                    }
                })?;
            tracing::debug!(method, url = %url.without_userinfo(), code = response.status, "Response code for request to DNS service.");
            let location = response.header("location").map(str::to_owned);
            if !matches!(response.status, 301 | 302 | 303 | 307 | 308)
                || location.as_deref().is_none_or(str::is_empty)
            {
                return Ok(response);
            }
            if request_number == 10 {
                return Err(Error::TooManyRedirects);
            }
            let next = url.resolve(location.as_deref().unwrap_or_default())?;
            if matches!(response.status, 301..=303) && method == "POST" {
                method = "GET";
                body = None;
                headers.retain(|h| {
                    !matches!(
                        h.name.to_ascii_lowercase().as_str(),
                        "content-encoding"
                            | "content-language"
                            | "content-location"
                            | "content-type"
                    )
                });
            }
            if !same_or_subdomain(next.host(), &initial_host) {
                headers.retain(|h| !is_sensitive(&h.name));
            }
            headers.retain(|h| !h.name.eq_ignore_ascii_case("referer"));
            if !(url.scheme().eq_ignore_ascii_case("https")
                && next.scheme().eq_ignore_ascii_case("http"))
            {
                headers.push(Header {
                    name: "Referer".into(),
                    value: url.referer(),
                });
            }
            url = next;
        }
        unreachable!()
    }

    fn decode<T: for<'de> Deserialize<'de>>(&self, response: Response) -> Result<T, Error> {
        if response.status == 401 {
            let auth: AuthErrorResponse =
                serde_json::from_slice(&response.body).map_err(Error::DecodeAuth)?;
            return Err(if auth.data.no_domain {
                Error::AuthNoDomain
            } else {
                Error::AuthenticationFailed
            });
        }
        if !(200..=300).contains(&response.status) {
            return Err(Error::UnexpectedStatus(response.status));
        }
        serde_json::from_slice(&response.body).map_err(|source| Error::DecodeResponse {
            body: response.body,
            source,
        })
    }
}

fn same_or_subdomain(host: &str, initial: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == initial
        || host
            .strip_suffix(initial)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn is_sensitive(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization"
            | "www-authenticate"
            | "cookie"
            | "cookie2"
            | "proxy-authorization"
            | "proxy-authenticate"
    )
}

fn go_json_line<T: serde::Serialize>(value: &T) -> Vec<u8> {
    let json = serde_json::to_string(value).expect("DNS wire values are always JSON serializable");
    let mut escaped = json
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
        .into_bytes();
    escaped.push(b'\n');
    escaped
}
