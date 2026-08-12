use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
};

use ployz_internal_dns::{
    Client, Header, RecordRequest, RecordResponse, RecordType, Request, Response, Transport,
    TransportError,
};

#[derive(Clone)]
struct Script {
    requests: Arc<Mutex<Vec<Request>>>,
    responses: Arc<Mutex<VecDeque<Result<Response, TransportError>>>>,
}

impl Script {
    fn new(responses: impl IntoIterator<Item = Response>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(responses.into_iter().map(Ok).collect())),
        }
    }
    fn from_results(responses: impl IntoIterator<Item = Result<Response, TransportError>>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
        }
    }
    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for Script {
    fn execute(&self, request: Request) -> Result<Response, TransportError> {
        self.requests.lock().unwrap().push(request);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected request")
    }
}

fn response(status: u16, body: &str) -> Response {
    Response {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

#[test]
fn reserves_domain_with_oracle_request_shape() {
    let script = Script::new([response(
        200,
        r#"{"name":"abc.uncld.dev","token":"secret"}"#,
    )]);
    let client = Client::with_transport(script.clone());
    assert_eq!(
        client.reserve_domain("http://EXAMPLE.test/v1").unwrap(),
        ("abc.uncld.dev".into(), "secret".into())
    );
    let request = &script.requests()[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.url, "http://EXAMPLE.test/v1/domains");
    assert!(request.body.is_none());
    assert_eq!(header(request, "content-type"), Some("application/json"));
    assert_eq!(header(request, "user-agent"), Some("Go-http-client/1.1"));
    assert_eq!(header(request, "accept-encoding"), Some("gzip"));
    assert_eq!(header(request, "authorization"), None);
}

#[test]
fn creates_records_sequentially_with_exact_json_and_partial_failure() {
    let script = Script::new([
        response(
            200,
            r#"{"name":"www","type":"A","values":["1.2.3.4"],"fqdn":"www.example"}"#,
        ),
        response(500, "failed"),
    ]);
    let client = Client::with_transport(script.clone());
    let records = [
        RecordRequest {
            name: "www".into(),
            record_type: RecordType::A,
            values: vec!["1.2.3.4".into()],
        },
        RecordRequest {
            name: String::new(),
            record_type: RecordType::Aaaa,
            values: Vec::new(),
        },
    ];
    let error = client
        .create_records("http://host/api", "example", "tok", &records)
        .unwrap_err();
    assert_eq!(error.completed().len(), 1);
    assert_eq!(error.to_string(), "unexpected response status code: 500");
    let requests = script.requests();
    assert_eq!(
        requests[0].body.as_deref(),
        Some(
            &br#"{"name":"www","type":"A","values":["1.2.3.4"]}
"#[..]
        )
    );
    assert_eq!(
        requests[1].body.as_deref(),
        Some(
            &br#"{"type":"AAAA"}
"#[..]
        )
    );
    assert_eq!(header(&requests[0], "authorization"), Some("Bearer tok"));
}

#[test]
fn request_json_uses_go_html_and_javascript_escaping() {
    let script = Script::new([response(200, r#"{}"#)]);
    Client::with_transport(script.clone())
        .create_records(
            "http://host",
            "d",
            "",
            &[RecordRequest {
                name: "<>&\u{2028}\u{2029}".into(),
                ..RecordRequest::default()
            }],
        )
        .unwrap();
    assert_eq!(
        script.requests()[0].body.as_deref(),
        Some(
            &br#"{"name":"\u003c\u003e\u0026\u2028\u2029"}
"#[..]
        )
    );
}

#[test]
fn endpoint_userinfo_becomes_basic_auth_but_bearer_takes_precedence() {
    let reserve = Script::new([response(200, r#"{}"#)]);
    Client::with_transport(reserve.clone())
        .reserve_domain("http://us%65r:pa%73s@host")
        .unwrap();
    assert_eq!(
        header(&reserve.requests()[0], "authorization"),
        Some("Basic dXNlcjpwYXNz")
    );
    let records = Script::new([response(200, r#"{}"#)]);
    Client::with_transport(records.clone())
        .create_records(
            "http://user:pass@host",
            "d",
            "token",
            &[RecordRequest::default()],
        )
        .unwrap();
    assert_eq!(
        header(&records.requests()[0], "authorization"),
        Some("Bearer token")
    );
}

#[test]
fn response_fields_are_folded_unknown_fields_ignored_and_later_duplicates_win() {
    let script = Script::new([response(
        200,
        r#"{"Name":"first","unknown":1,"name":"last","TOKEN":"t"}"#,
    )]);
    assert_eq!(
        Client::with_transport(script)
            .reserve_domain("http://host")
            .unwrap(),
        ("last".into(), "t".into())
    );
}

#[test]
fn json_null_matches_go_zero_value_and_duplicate_merge_behavior() {
    let script = Script::new([response(200, r#"{"name":"kept","name":null,"token":null}"#)]);
    assert_eq!(
        Client::with_transport(script)
            .reserve_domain("http://host")
            .unwrap(),
        ("kept".into(), String::new())
    );
    let auth = Script::new([response(401, r#"{"data":null}"#)]);
    assert_eq!(
        Client::with_transport(auth)
            .reserve_domain("http://host")
            .unwrap_err()
            .to_string(),
        "authentication failed"
    );
    let merged = Script::new([response(
        401,
        r#"{"status":2147483648,"data":{"noDomain":true,"noDomain":null},"data":{},"data":null}"#,
    )]);
    assert!(
        Client::with_transport(merged)
            .reserve_domain("http://host")
            .unwrap_err()
            .is_auth_no_domain()
    );
}

#[test]
fn empty_record_batch_does_not_parse_endpoint_or_make_requests() {
    let script = Script::new([]);
    assert!(
        Client::with_transport(script.clone())
            .create_records("not a URL", "domain", "token", &[])
            .unwrap()
            .is_empty()
    );
    assert!(script.requests().is_empty());
}

#[test]
fn status_and_auth_boundaries_match_oracle() {
    let no_domain = Script::new([response(401, r#"{"data":{"noDomain":true}}"#)]);
    assert!(
        Client::with_transport(no_domain)
            .reserve_domain("http://host")
            .unwrap_err()
            .is_auth_no_domain()
    );
    let generic = Script::new([response(401, r#"{"data":{}}"#)]);
    assert_eq!(
        Client::with_transport(generic)
            .reserve_domain("http://host")
            .unwrap_err()
            .to_string(),
        "authentication failed"
    );
    let malformed = Script::new([response(401, "not-json")]);
    assert!(
        Client::with_transport(malformed)
            .reserve_domain("http://host")
            .unwrap_err()
            .to_string()
            .starts_with("unmarshal auth error response:")
    );
    let status_300 = Script::new([response(300, r#"{"name":"ok"}"#)]);
    assert_eq!(
        Client::with_transport(status_300)
            .reserve_domain("http://host")
            .unwrap()
            .0,
        "ok"
    );
    let status_301_empty_location = Script::new([response(301, r#"{"name":"also-ok"}"#)]);
    assert_eq!(
        Client::with_transport(status_301_empty_location)
            .reserve_domain("http://host")
            .unwrap_err()
            .to_string(),
        "unexpected response status code: 301"
    );
}

#[test]
fn follows_redirects_with_method_header_and_credential_policy() {
    let first = Response {
        status: 302,
        headers: vec![Header {
            name: "Location".into(),
            value: "http://other.test/next".into(),
        }],
        body: b"ignored".to_vec(),
    };
    let script = Script::new([first, response(200, r#"{"name":"ok"}"#)]);
    let client = Client::with_transport(script.clone());
    client
        .create_records(
            "http://example.test",
            "d",
            "secret",
            &[RecordRequest::default()],
        )
        .unwrap();
    let requests = script.requests();
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[1].method, "GET");
    assert!(requests[1].body.is_none());
    assert_eq!(header(&requests[1], "authorization"), None);
    assert_eq!(header(&requests[1], "content-type"), None);
    assert_eq!(
        header(&requests[1], "referer"),
        Some("http://example.test/domains/d/records")
    );
}

#[test]
fn stops_before_an_eleventh_redirect_request() {
    let redirects = (0..10).map(|_| Response {
        status: 302,
        headers: vec![Header {
            name: "Location".into(),
            value: "/again".into(),
        }],
        body: Vec::new(),
    });
    let script = Script::new(redirects);
    assert_eq!(
        Client::with_transport(script.clone())
            .reserve_domain("http://host")
            .unwrap_err()
            .to_string(),
        "stopped after 10 redirects"
    );
    assert_eq!(script.requests().len(), 10);
}

#[test]
fn response_body_failures_keep_the_package_error_boundary() {
    let script = Script::from_results([Err(TransportError::response_body_error("broken body"))]);
    let error = Client::with_transport(script)
        .reserve_domain("http://host")
        .unwrap_err();
    assert_eq!(error.to_string(), "read response body: broken body");
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn record_response_keeps_unknown_record_types_for_future_callers() {
    let value: RecordResponse = serde_json::from_str(r#"{"type":"MX","fqdn":"x"}"#).unwrap();
    assert_eq!(value.record.record_type.as_str(), "MX");
}

#[test]
fn default_transport_executes_a_real_http_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let size = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..size]);
        assert!(request.starts_with("POST /domains HTTP/1.1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("user-agent: go-http-client/1.1\r\n")
        );
        let body = r#"{"name":"live","token":"transport"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    assert_eq!(
        Client::new()
            .reserve_domain(&format!("http://{address}"))
            .unwrap(),
        ("live".into(), "transport".into())
    );
    server.join().unwrap();
}

#[test]
fn default_transport_ignores_non_utf8_unrelated_response_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        let body = br#"{"name":"accepted"}"#;
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nX-Note: ",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(b"\x80\r\nConnection: close\r\n\r\n");
        response.extend_from_slice(body);
        stream.write_all(&response).unwrap();
    });
    assert_eq!(
        Client::new()
            .reserve_domain(&format!("http://{address}"))
            .unwrap()
            .0,
        "accepted"
    );
    server.join().unwrap();
}

#[test]
fn default_transport_does_not_serialize_unrelated_requests() {
    let slow = TcpListener::bind("127.0.0.1:0").unwrap();
    let slow_address = slow.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let slow_server = std::thread::spawn(move || {
        let (mut stream, _) = slow.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        accepted_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        let body = r#"{"name":"slow"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });

    let fast = TcpListener::bind("127.0.0.1:0").unwrap();
    let fast_address = fast.local_addr().unwrap();
    let fast_server = std::thread::spawn(move || {
        let (mut stream, _) = fast.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        let body = r#"{"name":"fast"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });

    let slow_call =
        std::thread::spawn(move || Client::new().reserve_domain(&format!("http://{slow_address}")));
    accepted_rx.recv().unwrap();
    assert_eq!(
        Client::new()
            .reserve_domain(&format!("http://{fast_address}"))
            .unwrap()
            .0,
        "fast"
    );
    release_tx.send(()).unwrap();
    assert_eq!(slow_call.join().unwrap().unwrap().0, "slow");
    slow_server.join().unwrap();
    fast_server.join().unwrap();
}

fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}
