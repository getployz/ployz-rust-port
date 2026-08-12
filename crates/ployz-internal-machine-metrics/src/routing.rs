use hyper::{Method, Request};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Route {
    Metrics,
    NotFound,
    Redirect(String),
    OptionsStar,
}

pub(crate) fn route<B>(request: &Request<B>) -> Route {
    let path = request.uri().path();
    if request.method() == Method::OPTIONS && path == "*" {
        return Route::OptionsStar;
    }

    if request.method() != Method::CONNECT {
        let cleaned = clean_path(path);
        if cleaned != path {
            let mut location = cleaned;
            if let Some(query) = request.uri().query() {
                location.push('?');
                location.push_str(query);
            }
            return Route::Redirect(location);
        }
    }

    if matches_metrics(path) {
        Route::Metrics
    } else {
        Route::NotFound
    }
}

fn matches_metrics(path: &str) -> bool {
    let Some(segment) = path.strip_prefix('/') else {
        return false;
    };
    !segment.contains('/')
        && percent_decode_segment(segment).is_some_and(|decoded| decoded == b"metrics")
}

fn percent_decode_segment(segment: &str) -> Option<Vec<u8>> {
    let bytes = segment.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = *bytes.get(index + 1)?;
            let low = *bytes.get(index + 2)?;
            output.push(hex(high)? << 4 | hex(low)?);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    Some(output)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn clean_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_owned();
    }
    let trailing_slash = path.ends_with('/');
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    let mut cleaned = format!("/{}", segments.join("/"));
    if trailing_slash && cleaned != "/" {
        cleaned.push('/');
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use hyper::Request;

    use super::*;

    fn request(method: &str, target: &str) -> Request<()> {
        Request::builder()
            .method(method)
            .uri(target)
            .body(())
            .unwrap()
    }

    #[test]
    fn matches_the_go_serve_mux_matrix() {
        for method in ["GET", "HEAD", "POST", "PUT", "OPTIONS", "CONNECT"] {
            assert_eq!(route(&request(method, "/metrics?x=1")), Route::Metrics);
        }
        assert_eq!(route(&request("GET", "/m%65trics")), Route::Metrics);
        for path in ["/metrics/", "/unrelated", "/metrics%2F", "/metrics%3Fx"] {
            assert_eq!(route(&request("GET", path)), Route::NotFound, "{path}");
        }
        assert_eq!(
            route(&request("GET", "/foo/../metrics?x=1")),
            Route::Redirect("/metrics?x=1".to_owned())
        );
        assert_eq!(
            route(&request("GET", "//metrics")),
            Route::Redirect("/metrics".to_owned())
        );
        assert_eq!(
            route(&request("GET", "/metrics/.")),
            Route::Redirect("/metrics".to_owned())
        );
        assert_eq!(
            route(&request("CONNECT", "/foo/../metrics?x=1")),
            Route::NotFound
        );
        assert_eq!(route(&request("OPTIONS", "*")), Route::OptionsStar);
    }
}
