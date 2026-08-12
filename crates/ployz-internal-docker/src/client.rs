use std::{cell::Cell, time::Duration};

use backon::Retryable as _;
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use bollard::{ClientVersion, Docker};
use futures_util::StreamExt as _;
use serde::Serialize;

use crate::{
    ApiError, ApiErrorKind, Cancellation, DaemonConfig, DockerError, ProgressStream,
    backoff::DockerBackoffBuilder, progress::progress_stream, reference::ImageReference,
    retrieve_local_registry_auth,
};

const ERROR_BODY_LIMIT: usize = 1024 * 1024;
const PORT_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug, Default)]
pub struct PullOptions {
    pub all: bool,
    pub registry_auth: Option<String>,
    pub platform: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PushOptions {
    pub all: bool,
    pub registry_auth: Option<String>,
    pub platform: Option<ImagePlatform>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ImagePlatform {
    pub architecture: String,
    pub os: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(rename = "os.version", skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(rename = "os.features", skip_serializing_if = "Vec::is_empty")]
    pub os_features: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessEvent {
    Waiting,
    Ready,
}

/// Docker client using Bollard generally and the approved raw seam only for image progress.
#[derive(Clone, Debug)]
pub struct Client {
    docker: Docker,
    raw: reqwest::Client,
    raw_base: String,
    version: ClientVersion,
}

impl Client {
    pub async fn connect(
        config: &DaemonConfig,
        cancellation: &Cancellation,
    ) -> Result<Self, DockerError> {
        let owned_config = config.clone();
        let (docker, (raw, raw_base)) = tokio::task::spawn_blocking(move || {
            Ok::<_, DockerError>((owned_config.bollard()?, owned_config.raw_client()?))
        })
        .await
        .map_err(DockerError::Task)??;
        let docker = if config.must_negotiate() {
            tokio::select! {
                biased;
                result = docker.negotiate_version() => result.map_err(DockerError::Engine)?,
                _ = cancellation.cancelled() => return Err(DockerError::Cancelled),
            }
        } else {
            docker
        };
        let version = docker.client_version();
        Ok(Self {
            docker,
            raw,
            raw_base,
            version,
        })
    }

    /// Reconstruct and ping until the daemon is ready. Cancellation is successful shutdown.
    pub async fn wait_until_ready<F>(
        config: &DaemonConfig,
        cancellation: &Cancellation,
        mut observe: F,
    ) -> Result<Option<Self>, DockerError>
    where
        F: FnMut(ReadinessEvent),
    {
        let waiting = Cell::new(false);
        let retry = (|| async {
            let client = Self::connect(config, cancellation).await?;
            tokio::select! {
                biased;
                result = client.docker.ping() => result.map_err(DockerError::Engine)?,
                _ = cancellation.cancelled() => return Err(DockerError::Cancelled),
            };
            Ok(client)
        })
        .retry(DockerBackoffBuilder::daemon_readiness())
        .when(DockerError::is_connection_failed)
        .notify(|_, _| {
            if !waiting.replace(true) {
                observe(ReadinessEvent::Waiting);
            }
        });
        let result = tokio::select! {
            biased;
            result = retry => result,
            _ = cancellation.cancelled() => return Ok(None),
        };
        match result {
            Ok(client) => {
                if waiting.get() {
                    observe(ReadinessEvent::Ready);
                }
                Ok(Some(client))
            }
            Err(DockerError::Cancelled) => Ok(None),
            Err(error) => Err(error
                .context("connect to Docker daemon")
                .context("ping Docker")),
        }
    }

    pub fn engine(&self) -> &Docker {
        &self.docker
    }

    pub fn api_version(&self) -> ClientVersion {
        self.version
    }

    pub async fn pull_image(
        &self,
        image: &str,
        mut options: PullOptions,
        cancellation: Cancellation,
    ) -> Result<ProgressStream, DockerError> {
        let reference = ImageReference::parse(image)?;
        let mut query = vec![("fromImage", reference.name.clone())];
        if !options.all
            && let Some(tag) = reference.api_tag()
        {
            query.push(("tag", tag.to_owned()));
        }
        if let Some(platform) = options.platform.take() {
            query.push(("platform", platform.to_ascii_lowercase()));
        }
        let auth = match options.registry_auth.take().filter(|auth| !auth.is_empty()) {
            Some(auth) => Some(auth),
            None => resolve_registry_auth(image).await?,
        };
        let request = self
            .raw
            .post(self.url("images/create"))
            .query(&query)
            .header("X-Registry-Auth", auth.unwrap_or_default());
        self.send_progress(request, cancellation).await
    }

    pub async fn push_image(
        &self,
        image: &str,
        mut options: PushOptions,
        cancellation: Cancellation,
    ) -> Result<ProgressStream, DockerError> {
        let reference = ImageReference::parse(image)?;
        if reference.digest.is_some() {
            return Err(DockerError::Configuration(
                "cannot push a digest reference".to_owned(),
            ));
        }
        let mut query = Vec::new();
        if !options.all {
            query.push((
                "tag",
                reference.tag.as_deref().unwrap_or("latest").to_owned(),
            ));
        }
        if let Some(platform) = options.platform.take() {
            if self.version
                < (ClientVersion {
                    major_version: 1,
                    minor_version: 46,
                })
            {
                return Err(DockerError::Configuration(
                    "Docker API 1.46 or newer is required to push a platform".to_owned(),
                ));
            }
            query.push((
                "platform",
                serde_json::to_string(&platform).map_err(|error| {
                    DockerError::Configuration(format!("encode push platform: {error}"))
                })?,
            ));
        }
        let auth = match options.registry_auth.take().filter(|auth| !auth.is_empty()) {
            Some(auth) => Some(auth),
            None => resolve_registry_auth(image).await?,
        }
        .unwrap_or_else(|| URL_SAFE.encode("{}"));
        let request = self
            .raw
            .post(self.url(&format!("images/{}/push", reference.name)))
            .query(&query)
            .header("X-Registry-Auth", auth)
            .header("Content-Type", "application/json")
            .body("{}\n");
        self.send_progress(request, cancellation).await
    }

    async fn send_progress(
        &self,
        request: reqwest::RequestBuilder,
        cancellation: Cancellation,
    ) -> Result<ProgressStream, DockerError> {
        let response = tokio::select! {
            biased;
            result = request.send() => result.map_err(DockerError::Connection)?,
            _ = cancellation.cancelled() => return Err(DockerError::Cancelled),
        };
        let response = check_response(response, &cancellation).await?;
        Ok(progress_stream(response, cancellation))
    }

    fn url(&self, operation: &str) -> String {
        format!("{}/v{}/{operation}", self.raw_base, self.version)
    }

    pub async fn create_container_with_image_pull(
        &self,
        name: &str,
        config: bollard::models::ContainerCreateBody,
        cancellation: Cancellation,
    ) -> Result<bollard::models::ContainerCreateResponse, DockerError> {
        let options = bollard::query_parameters::CreateContainerOptionsBuilder::default()
            .name(name)
            .build();
        let first = tokio::select! {
            biased;
            result = self.docker.create_container(Some(options.clone()), config.clone()) => result,
            _ = cancellation.cancelled() => return Err(DockerError::Cancelled),
        };
        match first {
            Ok(response) => return Ok(response),
            Err(error)
                if !matches!(
                    error,
                    bollard::errors::Error::DockerResponseServerError {
                        status_code: 404,
                        ..
                    }
                ) =>
            {
                return Err(DockerError::Engine(error));
            }
            Err(_) => {}
        }

        let image = config.image.as_deref().unwrap_or_default();
        let mut progress = self
            .pull_image(image, PullOptions::default(), cancellation.clone())
            .await
            .map_err(|error| error.context("pull image"))?;
        while let Some(item) = progress.next().await {
            if let Some(error) = item.error {
                return Err(DockerError::Progress(error).context("pull image"));
            }
        }

        tokio::select! {
            biased;
            result = self.docker.create_container(Some(options), config) => result.map_err(DockerError::Engine),
            _ = cancellation.cancelled() => Err(DockerError::Cancelled),
        }
    }

    pub async fn wait_port_published(
        &self,
        container: &str,
        port: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<bollard::models::PortBinding>, DockerError> {
        self.wait_port_published_with_timing(
            container,
            port,
            cancellation,
            Duration::from_secs(5),
            PORT_POLL_INTERVAL,
        )
        .await
    }

    async fn wait_port_published_with_timing(
        &self,
        container: &str,
        port: &str,
        cancellation: &Cancellation,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<Vec<bollard::models::PortBinding>, DockerError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let inspected = tokio::select! {
                biased;
                result = self.docker.inspect_container(container, None) => result
                    .map_err(DockerError::Engine)
                    .map_err(|error| error.context("inspect container"))?,
                _ = cancellation.cancelled() => {
                    return Err(DockerError::Cancelled.context("inspect container"));
                }
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(DockerError::DeadlineExceeded.context("inspect container"));
                }
            };
            if let Some(bindings) = inspected
                .network_settings
                .and_then(|settings| settings.ports)
                .and_then(|ports| ports.get(port).cloned().flatten())
                .filter(|bindings| !bindings.is_empty())
            {
                return Ok(bindings);
            }
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(DockerError::Cancelled),
                _ = tokio::time::sleep_until(deadline) => return Err(DockerError::Timeout),
                _ = tokio::time::sleep(poll_interval) => {}
            }
        }
    }
}

async fn check_response(
    mut response: reqwest::Response,
    cancellation: &Cancellation,
) -> Result<reqwest::Response, DockerError> {
    let status = response.status();
    if (200..400).contains(&status.as_u16()) {
        return Ok(response);
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let request_url = response.url().to_string();
    let mut body = Vec::new();
    loop {
        let chunk = tokio::select! {
            biased;
            chunk = response.chunk() => chunk.map_err(DockerError::Connection)?,
            _ = cancellation.cancelled() => return Err(DockerError::Cancelled),
        };
        let Some(chunk) = chunk else {
            break;
        };
        let remaining = ERROR_BODY_LIMIT.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if body.len() == ERROR_BODY_LIMIT {
            return Err(api_error(
                status.as_u16(),
                format!(
                    "request returned {status} with a message (> {ERROR_BODY_LIMIT} bytes) for API route and version {request_url}, check if the server supports the requested API version"
                ),
            ));
        }
    }
    if body.is_empty() {
        return Err(api_error(
            status.as_u16(),
            format!(
                "request returned {status} for API route and version {request_url}, check if the server supports the requested API version"
            ),
        ));
    }
    let message = if content_type.as_deref() == Some("application/json") {
        #[derive(serde::Deserialize)]
        struct ErrorResponse {
            #[serde(default)]
            message: String,
        }
        let parsed: ErrorResponse = serde_json::from_slice(&body)
            .map_err(|error| api_error(status.as_u16(), format!("Error reading JSON: {error}")))?;
        if parsed.message.is_empty() {
            format!(
                "Error response from daemon: API returned a {} ({}) but provided no error-message",
                status.as_u16(),
                status.canonical_reason().unwrap_or_default()
            )
        } else {
            format!("Error response from daemon: {}", parsed.message.trim())
        }
    } else {
        format!(
            "Error response from daemon: {}",
            String::from_utf8_lossy(&body).trim()
        )
    };
    Err(api_error(status.as_u16(), message))
}

fn api_error(status: u16, message: String) -> DockerError {
    let kind = match status {
        400 => ApiErrorKind::InvalidParameter,
        404 => ApiErrorKind::NotFound,
        409 => ApiErrorKind::Conflict,
        _ => ApiErrorKind::System,
    };
    DockerError::Api(ApiError {
        status,
        kind,
        message,
    })
}

async fn resolve_registry_auth(image: &str) -> Result<Option<String>, DockerError> {
    let image = image.to_owned();
    tokio::task::spawn_blocking(move || retrieve_local_registry_auth(&image).ok().flatten())
        .await
        .map_err(DockerError::Task)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        time::timeout,
    };

    async fn listen_once() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        (listener, url)
    }

    async fn read_headers(socket: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        loop {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                return request;
            }
        }
    }

    async fn write_chunk(socket: &mut TcpStream, bytes: &[u8]) {
        socket
            .write_all(format!("{:x}\r\n", bytes.len()).as_bytes())
            .await
            .unwrap();
        socket.write_all(bytes).await.unwrap();
        socket.write_all(b"\r\n").await.unwrap();
    }

    async fn test_client(url: String) -> Client {
        test_client_version(url, 53).await
    }

    async fn test_client_version(url: String, minor_version: usize) -> Client {
        Client::connect(
            &DaemonConfig::test_http(
                url,
                ClientVersion {
                    major_version: 1,
                    minor_version,
                },
            ),
            &Cancellation::new(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn pull_preserves_transcript_order_unknowns_null_and_embedded_error() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_headers(&mut socket).await).unwrap();
            assert!(request.starts_with(
                "POST /v1.53/images/create?fromImage=docker.io%2Frepo%2Fimage&tag=tag HTTP/1.1\r\n"
            ));
            assert!(request.contains("x-registry-auth: exact-token\r\n"));
            assert!(!request.contains("content-type:"));
            socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n").await.unwrap();
            for part in [
                &b"{\"id\":\"la"[..],
                &b"yer\",\"status\":\"Downloading\",\"progress\":\"2/4\",\"progressDetail\":{\"current\":2,\"total\":4,\"start\":1,\"hidecounts\":false,\"units\":\"B\",\"future\":7},\"futureTop\":true}null"[..],
                &b"{\"errorDetail\":{\"code\":500,\"message\":\"boom\",\"future\":true},\"error\":\"boom\"}"[..],
            ] {
                write_chunk(&mut socket, part).await;
            }
            socket.write_all(b"0\r\n\r\n").await.unwrap();
        });
        let client = test_client(url).await;
        let mut stream = client
            .pull_image(
                "repo/image:tag",
                PullOptions {
                    registry_auth: Some("exact-token".to_owned()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        let first = stream.next().await.unwrap();
        assert_eq!(first.message.id.as_deref(), Some("layer"));
        assert_eq!(first.message.progress.as_deref(), Some("2/4"));
        assert_eq!(
            first.message.progress_detail.as_ref().unwrap().start,
            Some(1)
        );
        assert_eq!(
            first.message.progress_detail.as_ref().unwrap().extra["future"],
            7
        );
        assert_eq!(first.message.extra["futureTop"], true);
        assert_eq!(stream.next().await.unwrap().message, Default::default());
        let embedded = stream.next().await.unwrap();
        assert!(matches!(
            embedded.error,
            Some(crate::ProgressError::Embedded(ref message)) if message == "boom"
        ));
        assert_eq!(
            embedded.message.error_detail.as_ref().unwrap().code,
            Some(500)
        );
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn push_sends_exact_body_platform_and_empty_auth_workaround() {
        let (listener, url) = listen_once().await;
        let expected_auth = URL_SAFE.encode("{}");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_headers(&mut socket).await).unwrap();
            assert!(request.starts_with("POST /v1.53/images/docker.io/acme/image/push?tag=tag&platform=%7B%22architecture%22%3A%22amd64%22%2C%22os%22%3A%22linux%22%2C%22os.version%22%3A%2212%22%2C%22os.features%22%3A%5B%22win32k%22%5D%7D HTTP/1.1\r\n"));
            assert!(request.contains(&format!("x-registry-auth: {expected_auth}\r\n")));
            assert!(request.contains("content-type: application/json\r\n"));
            assert!(request.contains("content-length: 3\r\n"));
            let mut body = [0; 3];
            socket.read_exact(&mut body).await.unwrap();
            assert_eq!(&body, b"{}\n");
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let client = test_client(url).await;
        let mut stream = client
            .push_image(
                "docker.io/acme/image:tag",
                PushOptions {
                    platform: Some(ImagePlatform {
                        architecture: "amd64".to_owned(),
                        os: "linux".to_owned(),
                        os_version: Some("12".to_owned()),
                        os_features: vec!["win32k".to_owned()],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn push_preserves_split_multiple_null_full_fields_and_embedded_error() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let mut body = [0; 3];
            socket.read_exact(&mut body).await.unwrap();
            assert_eq!(&body, b"{}\n");
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n",
                )
                .await
                .unwrap();
            for part in [
                &b"{\"id\":\"la"[..],
                &b"yer\",\"status\":\"Pushing\",\"progress\":\"2/4\",\"progressDetail\":{\"current\":2,\"total\":4,\"start\":1,\"hidecounts\":true,\"units\":\"B\",\"future\":7},\"stream\":\"out\",\"from\":\"daemon\",\"time\":42,\"timeNano\":43,\"aux\":{\"Digest\":\"sha256:abc\"},\"futureTop\":true}null"[..],
                &b"{\"errorDetail\":{\"code\":500,\"message\":\"boom\",\"future\":true},\"error\":\"boom\"}"[..],
            ] {
                write_chunk(&mut socket, part).await;
            }
            socket.write_all(b"0\r\n\r\n").await.unwrap();
        });
        let client = test_client(url).await;
        let mut stream = client
            .push_image(
                "acme/image:tag",
                PushOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        let first = stream.next().await.unwrap();
        assert_eq!(first.message.id.as_deref(), Some("layer"));
        assert_eq!(first.message.progress.as_deref(), Some("2/4"));
        assert_eq!(
            first.message.progress_detail.as_ref().unwrap().start,
            Some(1)
        );
        assert_eq!(
            first.message.progress_detail.as_ref().unwrap().extra["future"],
            7
        );
        assert_eq!(first.message.stream.as_deref(), Some("out"));
        assert_eq!(first.message.from.as_deref(), Some("daemon"));
        assert_eq!(first.message.time, Some(42));
        assert_eq!(first.message.time_nano, Some(43));
        assert_eq!(first.message.aux.as_ref().unwrap()["Digest"], "sha256:abc");
        assert_eq!(first.message.extra["futureTop"], true);
        assert_eq!(stream.next().await.unwrap().message, Default::default());
        let embedded = stream.next().await.unwrap();
        assert!(matches!(
            embedded.error,
            Some(crate::ProgressError::Embedded(ref message)) if message == "boom"
        ));
        assert_eq!(
            embedded.message.error_detail.as_ref().unwrap().code,
            Some(500)
        );
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn platform_push_is_rejected_before_io_below_api_1_46() {
        let (listener, url) = listen_once().await;
        let client = test_client_version(url, 45).await;
        let error = match client
            .push_image(
                "acme/image",
                PushOptions {
                    registry_auth: Some("auth".to_owned()),
                    platform: Some(ImagePlatform {
                        architecture: "amd64".to_owned(),
                        os: "linux".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
        {
            Ok(_) => panic!("platform push unexpectedly reached I/O below API 1.46"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("API 1.46 or newer"));
        assert!(
            timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn pull_all_uses_the_normalized_name_and_omits_tag_and_platform_is_lowercase() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_headers(&mut socket).await).unwrap();
            assert!(request.starts_with(
                "POST /v1.53/images/create?fromImage=docker.io%2Flibrary%2Fbusybox&platform=linux%2Farm64 HTTP/1.1\r\n"
            ));
            assert!(!request.lines().next().unwrap().contains("tag="));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let client = test_client(url).await;
        let mut stream = client
            .pull_image(
                "busybox:stable",
                PullOptions {
                    all: true,
                    registry_auth: Some("auth".to_owned()),
                    platform: Some("LINUX/ARM64".to_owned()),
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pull_without_credentials_sends_an_empty_auth_header() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_headers(&mut socket).await).unwrap();
            assert!(request.contains("x-registry-auth: \r\n"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let client = test_client(url).await;
        let mut stream = client
            .pull_image(
                "no-credentials.invalid/image",
                PullOptions {
                    registry_auth: Some(String::new()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pull_post_decode_cancellation_suppresses_an_embedded_error_with_bare_error() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let body = b"{\"status\":\"first\"}{\"errorDetail\":{\"message\":\"boom\"}}";
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len()).as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(body).await.unwrap();
        });
        let client = test_client(url).await;
        let cancellation = Cancellation::new();
        let mut stream = client
            .pull_image(
                "image",
                PullOptions {
                    registry_auth: Some(String::new()),
                    ..Default::default()
                },
                cancellation.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            stream.next().await.unwrap().message.status.as_deref(),
            Some("first")
        );
        cancellation.cancel();
        let cancelled = stream.next().await.unwrap();
        let error = cancelled.error.unwrap();
        assert!(matches!(error, crate::ProgressError::Cancelled(_)));
        assert_eq!(error.to_string(), "context canceled");
        assert!(std::error::Error::source(&error).is_some());
        assert_eq!(cancelled.message, Default::default());
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn push_post_decode_cancellation_suppresses_an_embedded_error_with_bare_error() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let mut request_body = [0; 3];
            socket.read_exact(&mut request_body).await.unwrap();
            let body = b"{\"status\":\"first\"}{\"errorDetail\":{\"message\":\"boom\"}}";
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len()).as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(body).await.unwrap();
        });
        let client = test_client(url).await;
        let cancellation = Cancellation::new();
        let mut stream = client
            .push_image(
                "acme/image",
                PushOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                cancellation.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            stream.next().await.unwrap().message.status.as_deref(),
            Some("first")
        );
        cancellation.cancel();
        let cancelled = stream.next().await.unwrap();
        assert!(matches!(
            cancelled.error,
            Some(crate::ProgressError::Cancelled(_))
        ));
        assert_eq!(cancelled.message, Default::default());
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pull_and_push_embedded_error_read_cancellation_races_have_only_oracle_outcomes() {
        use std::sync::Arc;
        use tokio::sync::Barrier;

        for push in [false, true] {
            let (listener, url) = listen_once().await;
            let barrier = Arc::new(Barrier::new(2));
            let server_barrier = barrier.clone();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_headers(&mut socket).await;
                if push {
                    let mut request_body = [0; 3];
                    socket.read_exact(&mut request_body).await.unwrap();
                }
                let body = br#"{"error":"boom","errorDetail":{"message":"boom"}}"#;
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                server_barrier.wait().await;
                let _ = socket.write_all(body).await;
            });
            let client = test_client(url).await;
            let cancellation = Cancellation::new();
            let mut stream = if push {
                client
                    .push_image(
                        "acme/image",
                        PushOptions {
                            registry_auth: Some("auth".to_owned()),
                            ..Default::default()
                        },
                        cancellation.clone(),
                    )
                    .await
                    .unwrap()
            } else {
                client
                    .pull_image(
                        "acme/image",
                        PullOptions {
                            registry_auth: Some("auth".to_owned()),
                            ..Default::default()
                        },
                        cancellation.clone(),
                    )
                    .await
                    .unwrap()
            };
            let cancel = cancellation.clone();
            let cancel_barrier = barrier.clone();
            let canceller = tokio::spawn(async move {
                cancel_barrier.wait().await;
                cancel.cancel();
            });
            let error = stream.next().await.unwrap().error.unwrap();
            assert!(matches!(
                error,
                crate::ProgressError::Embedded(_)
                    | crate::ProgressError::DecodeCancelled(_)
                    | crate::ProgressError::Cancelled(_)
            ));
            drop(stream);
            canceller.await.unwrap();
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn cancellation_during_body_read_is_wrapped_and_closes_connection() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            let mut byte = [0];
            let closed = timeout(Duration::from_secs(2), socket.read(&mut byte)).await;
            assert!(matches!(closed, Ok(Ok(0))));
        });
        let client = test_client(url).await;
        let cancellation = Cancellation::new();
        let mut stream = client
            .pull_image(
                "image",
                PullOptions {
                    registry_auth: Some(String::new()),
                    ..Default::default()
                },
                cancellation.clone(),
            )
            .await
            .unwrap();
        let cancel = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel.cancel();
        });
        let item = stream.next().await.unwrap();
        let error = item.error.unwrap();
        assert!(matches!(error, crate::ProgressError::DecodeCancelled(_)));
        assert_eq!(
            error.to_string(),
            "decode image pull/push message: context canceled"
        );
        assert!(std::error::Error::source(&error).is_some());
        drop(stream);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn push_cancellation_during_body_read_is_wrapped_and_closes_connection() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let mut body = [0; 3];
            socket.read_exact(&mut body).await.unwrap();
            assert_eq!(&body, b"{}\n");
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            let mut byte = [0];
            let closed = timeout(Duration::from_secs(2), socket.read(&mut byte)).await;
            assert!(matches!(closed, Ok(Ok(0))));
        });
        let client = test_client(url).await;
        let cancellation = Cancellation::new();
        let mut stream = client
            .push_image(
                "acme/image",
                PushOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                cancellation.clone(),
            )
            .await
            .unwrap();
        cancellation.cancel();
        let error = stream.next().await.unwrap().error.unwrap();
        assert!(matches!(error, crate::ProgressError::DecodeCancelled(_)));
        drop(stream);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn push_cancellation_while_waiting_for_headers_is_outer_error_and_closes_connection() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let mut body = [0; 3];
            socket.read_exact(&mut body).await.unwrap();
            let mut byte = [0];
            let closed = timeout(Duration::from_secs(2), socket.read(&mut byte)).await;
            assert!(matches!(closed, Ok(Ok(0))));
        });
        let client = test_client(url).await;
        let cancellation = Cancellation::new();
        let cancel = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel.cancel();
        });
        let error = match client
            .push_image(
                "acme/image",
                PushOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                cancellation,
            )
            .await
        {
            Ok(_) => panic!("cancelled push header wait unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(matches!(error, DockerError::Cancelled));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_final_json_yields_one_wrapped_error_then_eof() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let body = b"{\"status\":\"partial";
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(body).await.unwrap();
        });
        let client = test_client(url).await;
        let mut stream = client
            .pull_image(
                "image",
                PullOptions {
                    registry_auth: Some(String::new()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        let error = stream.next().await.unwrap().error.unwrap();
        assert!(matches!(error, crate::ProgressError::DecodeIo(_)));
        assert_eq!(
            error.to_string(),
            "decode image pull/push message: unexpected EOF"
        );
        assert!(std::error::Error::source(&error).is_some());
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn push_malformed_final_json_yields_one_wrapped_error_then_eof() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let mut request_body = [0; 3];
            socket.read_exact(&mut request_body).await.unwrap();
            let body = b"{\"status\":\"partial";
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(body).await.unwrap();
        });
        let client = test_client(url).await;
        let mut stream = client
            .push_image(
                "acme/image",
                PushOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        let error = stream.next().await.unwrap().error.unwrap();
        assert!(matches!(error, crate::ProgressError::DecodeIo(_)));
        assert!(std::error::Error::source(&error).is_some());
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dropping_a_progress_consumer_closes_the_response_immediately() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            write_chunk(&mut socket, br#"{"status":"first"}"#).await;
            let mut byte = [0];
            let closed = timeout(Duration::from_secs(2), socket.read(&mut byte)).await;
            assert!(matches!(closed, Ok(Ok(0))));
        });
        let client = test_client(url).await;
        let mut stream = client
            .pull_image(
                "image",
                PullOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            stream.next().await.unwrap().message.status.as_deref(),
            Some("first")
        );
        drop(stream);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dropping_a_push_progress_consumer_closes_the_response_immediately() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let mut request_body = [0; 3];
            socket.read_exact(&mut request_body).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            write_chunk(&mut socket, br#"{"status":"first"}"#).await;
            let mut byte = [0];
            let closed = timeout(Duration::from_secs(2), socket.read(&mut byte)).await;
            assert!(matches!(closed, Ok(Ok(0))));
        });
        let client = test_client(url).await;
        let mut stream = client
            .push_image(
                "acme/image",
                PushOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            stream.next().await.unwrap().message.status.as_deref(),
            Some("first")
        );
        drop(stream);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dropping_after_an_embedded_error_closes_the_response_immediately() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            write_chunk(
                &mut socket,
                br#"{"error":"boom","errorDetail":{"message":"boom"}}"#,
            )
            .await;
            let mut byte = [0];
            let closed = timeout(Duration::from_secs(2), socket.read(&mut byte)).await;
            assert!(matches!(closed, Ok(Ok(0))));
        });
        let client = test_client(url).await;
        let mut stream = client
            .pull_image(
                "image",
                PullOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert!(matches!(
            stream.next().await.unwrap().error,
            Some(crate::ProgressError::Embedded(ref message)) if message == "boom"
        ));
        drop(stream);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn truncated_http_body_preserves_the_transport_error_source() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\n{\"status\":")
                .await
                .unwrap();
        });
        let client = test_client(url).await;
        let mut stream = client
            .pull_image(
                "image",
                PullOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        let error = stream.next().await.unwrap().error.unwrap();
        assert!(matches!(error, crate::ProgressError::DecodeTransport(_)));
        assert!(std::error::Error::source(&error).is_some());
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn truncated_push_body_preserves_the_transport_error_source() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let mut request_body = [0; 3];
            socket.read_exact(&mut request_body).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\n{\"status\":")
                .await
                .unwrap();
        });
        let client = test_client(url).await;
        let mut stream = client
            .push_image(
                "acme/image",
                PushOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        let error = stream.next().await.unwrap().error.unwrap();
        assert!(matches!(error, crate::ProgressError::DecodeTransport(_)));
        assert!(std::error::Error::source(&error).is_some());
        assert!(stream.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn redirect_with_location_is_connection_error_but_without_location_is_accepted() {
        async fn redirect(location: bool) -> Result<ProgressStream, DockerError> {
            let (listener, url) = listen_once().await;
            tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_headers(&mut socket).await;
                let location = if location {
                    "location: http://127.0.0.1/elsewhere\r\n"
                } else {
                    ""
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 307 Temporary Redirect\r\n{location}content-length: 0\r\nconnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            });
            test_client(url)
                .await
                .pull_image(
                    "image",
                    PullOptions {
                        registry_auth: Some(String::new()),
                        ..Default::default()
                    },
                    Cancellation::new(),
                )
                .await
        }

        let error = match redirect(true).await {
            Ok(_) => panic!("redirect unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(error.is_connection_failed());
        let mut source = Some(&error as &dyn std::error::Error);
        let mut found_redirect = false;
        while let Some(error) = source {
            found_redirect |= error.to_string().contains("unexpected redirect");
            source = error.source();
        }
        assert!(found_redirect);
        let mut accepted = redirect(false).await.unwrap();
        assert!(accepted.next().await.is_none());
    }

    #[tokio::test]
    async fn cancellation_while_waiting_for_headers_is_outer_error_and_closes_connection() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let mut byte = [0];
            let closed = timeout(Duration::from_secs(2), socket.read(&mut byte)).await;
            assert!(matches!(closed, Ok(Ok(0))));
        });
        let client = test_client(url).await;
        let cancellation = Cancellation::new();
        let cancel = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel.cancel();
        });
        let error = match client
            .pull_image(
                "image",
                PullOptions {
                    registry_auth: Some(String::new()),
                    ..Default::default()
                },
                cancellation,
            )
            .await
        {
            Ok(_) => panic!("cancelled header wait unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(matches!(error, DockerError::Cancelled));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_while_reading_daemon_error_is_outer_error_and_closes_connection() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            socket
                .write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\ntransfer-encoding: chunked\r\n\r\n",
                )
                .await
                .unwrap();
            let mut byte = [0];
            let closed = timeout(Duration::from_secs(2), socket.read(&mut byte)).await;
            assert!(matches!(closed, Ok(Ok(0))));
        });
        let client = test_client(url).await;
        let cancellation = Cancellation::new();
        let cancel = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel.cancel();
        });
        let error = match client
            .pull_image(
                "image",
                PullOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                cancellation,
            )
            .await
        {
            Ok(_) => panic!("cancelled daemon error read unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(matches!(error, DockerError::Cancelled));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn informational_final_response_is_not_accepted() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nconnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let client = test_client(url).await;
        let error = match client
            .pull_image(
                "image",
                PullOptions {
                    registry_auth: Some("auth".to_owned()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
        {
            Ok(_) => panic!("informational response unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            DockerError::Api(ApiError { status: 101, .. })
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn daemon_json_plain_empty_and_malformed_errors_remain_distinct() {
        async fn response_error(status_line: &str, content_type: &str, body: &[u8]) -> DockerError {
            let (listener, url) = listen_once().await;
            let status_line = status_line.to_owned();
            let content_type = content_type.to_owned();
            let body = body.to_vec();
            tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_headers(&mut socket).await;
                socket.write_all(format!("HTTP/1.1 {status_line}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            });
            match test_client(url)
                .await
                .pull_image(
                    "image",
                    PullOptions {
                        registry_auth: Some(String::new()),
                        ..Default::default()
                    },
                    Cancellation::new(),
                )
                .await
            {
                Ok(_) => panic!("error response unexpectedly succeeded"),
                Err(error) => error,
            }
        }

        let json = response_error(
            "404 Not Found",
            "application/json",
            br#"{"message":" missing "}"#,
        )
        .await;
        assert!(json.is_not_found());
        assert_eq!(json.to_string(), "Error response from daemon: missing");
        let plain = response_error("500 Internal Server Error", "text/plain", b" broken \n").await;
        assert_eq!(plain.to_string(), "Error response from daemon: broken");
        let empty = response_error("500 Internal Server Error", "text/plain", b"").await;
        assert!(empty.to_string().contains("check if the server supports"));
        let malformed = response_error("500 Internal Server Error", "application/json", b"{").await;
        assert!(malformed.to_string().starts_with("Error reading JSON:"));

        let at_limit = vec![b'x'; ERROR_BODY_LIMIT];
        let at_limit = response_error("500 Internal Server Error", "text/plain", &at_limit).await;
        assert!(at_limit.to_string().contains("message (> 1048576 bytes)"));
        let over_limit = vec![b'x'; ERROR_BODY_LIMIT + 1];
        let over_limit =
            response_error("500 Internal Server Error", "text/plain", &over_limit).await;
        assert!(over_limit.to_string().contains("message (> 1048576 bytes)"));
    }

    #[tokio::test]
    async fn missing_image_create_pulls_to_completion_then_retries_once() {
        use std::sync::{Arc, Mutex};

        let (listener, url) = listen_once().await;
        let paths = Arc::new(Mutex::new(Vec::new()));
        let observed = paths.clone();
        let server = tokio::spawn(async move {
            for step in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = String::from_utf8(read_headers(&mut socket).await).unwrap();
                let path = request.lines().next().unwrap().to_owned();
                observed.lock().unwrap().push(path);
                let (status, body) = match step {
                    0 => ("404 Not Found", r#"{"message":"No such image"}"#),
                    1 => ("200 OK", ""),
                    2 => ("201 Created", r#"{"Id":"created","Warnings":[]}"#),
                    _ => unreachable!(),
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let client = test_client(url).await;
        let response = client
            .create_container_with_image_pull(
                "proxy",
                bollard::models::ContainerCreateBody {
                    image: Some("alpine/socat:1.8.0.3".to_owned()),
                    ..Default::default()
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert_eq!(response.id, "created");
        server.await.unwrap();
        let paths = paths.lock().unwrap();
        assert!(
            paths[0].contains("/containers/create?name=proxy"),
            "{paths:?}"
        );
        assert_eq!(
            paths[1],
            "POST /v1.53/images/create?fromImage=docker.io%2Falpine%2Fsocat&tag=1.8.0.3 HTTP/1.1"
        );
        assert!(
            paths[2].contains("/containers/create?name=proxy"),
            "{paths:?}"
        );
    }

    #[tokio::test]
    async fn wait_port_wraps_inspect_failures() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_headers(&mut socket).await).unwrap();
            assert!(
                request
                    .lines()
                    .next()
                    .unwrap()
                    .contains("/containers/id/json")
            );
            let body = r#"{"message":"gone"}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let client = test_client(url).await;
        let error = client
            .wait_port_published("id", "80/tcp", &Cancellation::new())
            .await
            .unwrap_err();
        assert!(error.to_string().starts_with("inspect container: "));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn wait_port_distinguishes_inspect_deadline_from_delay_timeout() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            let mut byte = [0];
            let closed = timeout(Duration::from_secs(1), socket.read(&mut byte)).await;
            assert!(matches!(closed, Ok(Ok(0))));
        });
        let client = test_client(url).await;
        let inspect_deadline = client
            .wait_port_published_with_timing(
                "id",
                "80/tcp",
                &Cancellation::new(),
                Duration::from_millis(30),
                PORT_POLL_INTERVAL,
            )
            .await
            .unwrap_err();
        assert_eq!(
            inspect_deadline.to_string(),
            "inspect container: context deadline exceeded"
        );
        server.await.unwrap();

        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")
                .await
                .unwrap();
        });
        let client = test_client(url).await;
        let delay_timeout = client
            .wait_port_published_with_timing(
                "id",
                "80/tcp",
                &Cancellation::new(),
                Duration::from_millis(5),
                PORT_POLL_INTERVAL,
            )
            .await
            .unwrap_err();
        assert_eq!(delay_timeout.to_string(), "timeout");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn wait_port_reinspects_during_the_final_partial_poll_interval() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            for inspection in 0..4 {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_headers(&mut socket).await;
                let body = if inspection == 3 {
                    r#"{"NetworkSettings":{"Ports":{"80/tcp":[{"HostIp":"127.0.0.1","HostPort":"8080"}]}}}"#
                } else {
                    "{}"
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let client = test_client(url).await;
        let bindings = client
            .wait_port_published_with_timing(
                "id",
                "80/tcp",
                &Cancellation::new(),
                Duration::from_millis(350),
                Duration::from_millis(100),
            )
            .await
            .unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].host_port.as_deref(), Some("8080"));
        server.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn readiness_cancellation_while_socket_is_missing_is_successful_shutdown() {
        let path = std::env::temp_dir().join(format!(
            "ployz-internal-docker-missing-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let cancellation = Cancellation::new();
        let cancel = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.cancel();
        });
        let mut events = Vec::new();
        let result = Client::wait_until_ready(&DaemonConfig::unix(path), &cancellation, |event| {
            events.push(event)
        })
        .await
        .unwrap();
        assert!(result.is_none());
        assert_eq!(events, [ReadinessEvent::Waiting]);
    }

    #[tokio::test]
    async fn readiness_reports_waiting_once_and_ready_only_after_recovery() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            read_headers(&mut first).await;
            drop(first);

            let (mut second, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_headers(&mut second).await).unwrap();
            assert!(request.starts_with("GET /_ping HTTP/1.1\r\n"));
            second
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nOK")
                .await
                .unwrap();
        });
        let config = DaemonConfig::test_http(
            url,
            ClientVersion {
                major_version: 1,
                minor_version: 53,
            },
        );
        let mut events = Vec::new();
        let ready =
            Client::wait_until_ready(&config, &Cancellation::new(), |event| events.push(event))
                .await
                .unwrap();
        assert!(ready.is_some());
        assert_eq!(events, [ReadinessEvent::Waiting, ReadinessEvent::Ready]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn immediate_readiness_emits_no_state_change() {
        let (listener, url) = listen_once().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nOK")
                .await
                .unwrap();
        });
        let config = DaemonConfig::test_http(
            url,
            ClientVersion {
                major_version: 1,
                minor_version: 53,
            },
        );
        let mut events = Vec::new();
        let ready =
            Client::wait_until_ready(&config, &Cancellation::new(), |event| events.push(event))
                .await
                .unwrap();
        assert!(ready.is_some());
        assert!(events.is_empty());
        server.await.unwrap();
    }
}
