use std::{
    error, fmt,
    io::Read,
    sync::{OnceLock, mpsc},
    thread,
};

use bytes::Bytes;
use flate2::read::GzDecoder;
use http_body_util::{BodyExt, Full};
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<Header>,
    pub body: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }
}

#[derive(Debug)]
pub struct TransportError {
    phase: TransportPhase,
    source: Box<dyn error::Error + Send + Sync>,
}

impl TransportError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::request(std::io::Error::other(message.into()))
    }

    fn request(source: impl error::Error + Send + Sync + 'static) -> Self {
        Self {
            phase: TransportPhase::Request,
            source: Box::new(source),
        }
    }

    fn response_body(source: impl error::Error + Send + Sync + 'static) -> Self {
        Self {
            phase: TransportPhase::ResponseBody,
            source: Box::new(source),
        }
    }

    fn protocol(source: impl error::Error + Send + Sync + 'static) -> Self {
        Self {
            phase: TransportPhase::Protocol,
            source: Box::new(source),
        }
    }

    pub fn response_body_error(message: impl Into<String>) -> Self {
        Self::response_body(std::io::Error::other(message.into()))
    }

    pub fn is_response_body(&self) -> bool {
        self.phase == TransportPhase::ResponseBody
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportPhase {
    Request,
    Protocol,
    ResponseBody,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}
impl error::Error for TransportError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub trait Transport: Send + Sync + 'static {
    fn execute(&self, request: Request) -> Result<Response, TransportError>;
}

type Reply = mpsc::Sender<Result<Response, TransportError>>;
struct Job {
    request: Request,
    reply: Reply,
}

#[derive(Clone)]
pub(crate) struct DefaultTransport {
    sender: mpsc::Sender<Job>,
}

impl DefaultTransport {
    pub(crate) fn shared() -> Self {
        static SENDER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
        Self {
            sender: SENDER.get_or_init(spawn_worker).clone(),
        }
    }
}

fn spawn_worker() -> mpsc::Sender<Job> {
    let (sender, receiver) = mpsc::channel::<Job>();
    thread::Builder::new()
        .name("ployz-dns-http".into())
        .spawn(move || run_worker(receiver))
        .expect("failed to start DNS HTTP worker");
    sender
}

fn run_worker(receiver: mpsc::Receiver<Job>) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .expect("failed to start DNS HTTP runtime");
    let connector = HttpsConnectorBuilder::new()
        .with_platform_verifier()
        .https_or_http()
        .enable_all_versions()
        .build();
    let client: Client<_, Full<Bytes>> = Client::builder(TokioExecutor::new())
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(2)
        .build(connector);
    while let Ok(job) = receiver.recv() {
        let client = client.clone();
        runtime.spawn(async move {
            let result = execute(&client, job.request).await;
            let _ = job.reply.send(result);
        });
    }
}

async fn execute<C>(
    client: &Client<C, Full<Bytes>>,
    request: Request,
) -> Result<Response, TransportError>
where
    C: hyper_util::client::legacy::connect::Connect + Clone + Send + Sync + 'static,
{
    let mut builder = hyper::Request::builder()
        .method(request.method.as_str())
        .uri(request.url.as_str());
    for header in &request.headers {
        builder = builder.header(header.name.as_str(), header.value.as_str());
    }
    let body = Full::new(Bytes::from(request.body.unwrap_or_default()));
    let request = builder.body(body).map_err(TransportError::request)?;
    let response = client
        .request(request)
        .await
        .map_err(TransportError::request)?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter(|(name, _)| {
            name.as_str().eq_ignore_ascii_case("location")
                || name.as_str().eq_ignore_ascii_case("content-encoding")
        })
        .map(|(name, value)| {
            value
                .to_str()
                .map_err(|error| {
                    TransportError::protocol(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid {name} response header: {error}"),
                    ))
                })
                .map(|value| Header {
                    name: name.as_str().to_owned(),
                    value: value.to_owned(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut body = response
        .into_body()
        .collect()
        .await
        .map_err(TransportError::response_body)?
        .to_bytes()
        .to_vec();
    if headers.iter().any(|h| {
        h.name.eq_ignore_ascii_case("content-encoding") && h.value.eq_ignore_ascii_case("gzip")
    }) {
        let mut decoded = Vec::new();
        GzDecoder::new(body.as_slice())
            .read_to_end(&mut decoded)
            .map_err(TransportError::response_body)?;
        body = decoded;
    }
    Ok(Response {
        status,
        headers,
        body,
    })
}

impl Transport for DefaultTransport {
    fn execute(&self, request: Request) -> Result<Response, TransportError> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .send(Job {
                request,
                reply: sender,
            })
            .map_err(|_| TransportError::new("DNS HTTP worker stopped"))?;
        receiver
            .recv()
            .map_err(|_| TransportError::new("DNS HTTP worker stopped"))?
    }
}
