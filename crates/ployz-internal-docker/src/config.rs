use std::{env, fs, path::PathBuf, sync::OnceLock, time::Duration};

use bollard::{API_DEFAULT_VERSION, ClientVersion, Docker};

use crate::{CryptoProviderConflict, DockerError};

const BOLLARD_HEADER_TIMEOUT_SECONDS: u64 = 120;

#[derive(Clone, Debug)]
enum Endpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    #[cfg(windows)]
    NamedPipe(String),
    Tls {
        host: String,
        certificates: PathBuf,
    },
    #[cfg(test)]
    TestHttp(String),
}

/// Canonical Docker daemon configuration shared by Bollard and the raw image seam.
#[derive(Clone, Debug)]
pub struct DaemonConfig {
    endpoint: Endpoint,
    requested_version: Option<ClientVersion>,
}

impl DaemonConfig {
    pub fn from_env() -> Result<Self, DockerError> {
        let host = env::var("DOCKER_HOST").unwrap_or_else(|_| default_host().to_owned());
        let requested_version = env::var("DOCKER_API_VERSION")
            .ok()
            .and_then(|value| parse_version(value.strip_prefix('v').unwrap_or(&value)))
            .transpose()?;

        #[cfg(unix)]
        if let Some(path) = host.strip_prefix("unix://") {
            return Ok(Self {
                endpoint: Endpoint::Unix(PathBuf::from(path)),
                requested_version,
            });
        }
        #[cfg(windows)]
        if host.starts_with("npipe://") {
            return Ok(Self {
                endpoint: Endpoint::NamedPipe(host),
                requested_version,
            });
        }

        if host.starts_with("tcp://") || host.starts_with("https://") {
            let verify = env::var_os("DOCKER_TLS_VERIFY").is_some_and(|value| !value.is_empty());
            let Some(certificates) = env::var_os("DOCKER_CERT_PATH").map(PathBuf::from) else {
                return Err(DockerError::Configuration(
                    "remote Docker daemons require DOCKER_TLS_VERIFY and DOCKER_CERT_PATH; plaintext and unverified TLS are unsupported".to_owned(),
                ));
            };
            if !verify {
                return Err(DockerError::Configuration(
                    "remote Docker daemons require DOCKER_TLS_VERIFY and DOCKER_CERT_PATH; plaintext and unverified TLS are unsupported".to_owned(),
                ));
            }
            return Ok(Self {
                endpoint: Endpoint::Tls { host, certificates },
                requested_version,
            });
        }

        Err(DockerError::Configuration(format!(
            "unsupported DOCKER_HOST transport: {host}"
        )))
    }

    #[cfg(unix)]
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        Self {
            endpoint: Endpoint::Unix(path.into()),
            requested_version: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_http(host: String, version: ClientVersion) -> Self {
        Self {
            endpoint: Endpoint::TestHttp(host),
            requested_version: Some(version),
        }
    }

    pub(crate) fn requested_version(&self) -> ClientVersion {
        self.requested_version.unwrap_or(*API_DEFAULT_VERSION)
    }

    pub(crate) fn must_negotiate(&self) -> bool {
        self.requested_version.is_none()
    }

    pub(crate) fn bollard(&self) -> Result<Docker, DockerError> {
        install_crypto_provider()?;
        let version = self.requested_version();
        match &self.endpoint {
            #[cfg(unix)]
            Endpoint::Unix(path) => Docker::connect_with_unix(
                path.to_string_lossy().as_ref(),
                BOLLARD_HEADER_TIMEOUT_SECONDS,
                &version,
            )
            .map_err(DockerError::Engine),
            #[cfg(windows)]
            Endpoint::NamedPipe(path) => {
                Docker::connect_with_named_pipe(path, BOLLARD_HEADER_TIMEOUT_SECONDS, &version)
                    .map_err(DockerError::Engine)
            }
            Endpoint::Tls { host, certificates } => Docker::connect_with_ssl(
                host,
                &certificates.join("key.pem"),
                &certificates.join("cert.pem"),
                &certificates.join("ca.pem"),
                BOLLARD_HEADER_TIMEOUT_SECONDS,
                &version,
            )
            .map_err(DockerError::Engine),
            #[cfg(test)]
            Endpoint::TestHttp(host) => {
                Docker::connect_with_http(host, BOLLARD_HEADER_TIMEOUT_SECONDS, &version)
                    .map_err(DockerError::Engine)
            }
        }
    }

    pub(crate) fn raw_client(&self) -> Result<(reqwest::Client, String), DockerError> {
        install_crypto_provider()?;
        let builder = reqwest::Client::builder()
            .http1_only()
            .no_proxy()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                attempt.error(UnexpectedRedirect)
            }))
            .connect_timeout(Duration::from_secs(10));
        #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
        let builder = builder.tcp_user_timeout(None::<Duration>);

        let (builder, base) = match &self.endpoint {
            #[cfg(unix)]
            Endpoint::Unix(path) => (
                builder.unix_socket(path.as_path()),
                "http://api.moby.localhost".to_owned(),
            ),
            #[cfg(windows)]
            Endpoint::NamedPipe(path) => (
                builder.windows_named_pipe(path.as_str()),
                "http://api.moby.localhost".to_owned(),
            ),
            Endpoint::Tls { host, certificates } => {
                let mut builder = builder;
                let ca = fs::read(certificates.join("ca.pem")).map_err(|error| {
                    DockerError::Configuration(format!("read Docker CA certificate: {error}"))
                })?;
                for certificate in reqwest::Certificate::from_pem_bundle(&ca).map_err(|error| {
                    DockerError::Configuration(format!("parse Docker CA certificate: {error}"))
                })? {
                    builder = builder.add_root_certificate(certificate);
                }
                let mut identity = fs::read(certificates.join("cert.pem")).map_err(|error| {
                    DockerError::Configuration(format!("read Docker client certificate: {error}"))
                })?;
                identity.extend_from_slice(&fs::read(certificates.join("key.pem")).map_err(
                    |error| DockerError::Configuration(format!("read Docker client key: {error}")),
                )?);
                let identity = reqwest::Identity::from_pem(&identity).map_err(|error| {
                    DockerError::Configuration(format!("parse Docker client identity: {error}"))
                })?;
                let base = host.replacen("tcp://", "https://", 1);
                (
                    builder.identity(identity),
                    base.trim_end_matches('/').to_owned(),
                )
            }
            #[cfg(test)]
            Endpoint::TestHttp(host) => (builder, host.trim_end_matches('/').to_owned()),
        };
        builder
            .build()
            .map(|client| (client, base))
            .map_err(|error| {
                DockerError::Configuration(format!("build Docker HTTP client: {error}"))
            })
    }
}

#[cfg(unix)]
fn default_host() -> &'static str {
    "unix:///var/run/docker.sock"
}

#[cfg(windows)]
fn default_host() -> &'static str {
    "npipe:////./pipe/docker_engine"
}

fn parse_version(value: &str) -> Option<Result<ClientVersion, DockerError>> {
    if value.is_empty() {
        return None;
    }
    let mut parts = value.split('.');
    let parsed = parts
        .next()
        .and_then(|major| major.parse::<usize>().ok())
        .zip(parts.next().and_then(|minor| minor.parse::<usize>().ok()));
    let Some((major_version, minor_version)) = parsed else {
        return Some(Err(DockerError::Configuration(format!(
            "invalid DOCKER_API_VERSION: {value}"
        ))));
    };
    if parts.next().is_some() {
        return Some(Err(DockerError::Configuration(format!(
            "invalid DOCKER_API_VERSION: {value}"
        ))));
    }
    Some(Ok(ClientVersion {
        major_version,
        minor_version,
    }))
}

fn install_crypto_provider() -> Result<(), DockerError> {
    static INSTALLED: OnceLock<Result<(), CryptoProviderConflict>> = OnceLock::new();
    (*INSTALLED.get_or_init(|| {
        if let Some(provider) = rustls::crypto::CryptoProvider::get_default() {
            return ring_compatible(provider)
                .then_some(())
                .ok_or(CryptoProviderConflict);
        }

        if rustls::crypto::ring::default_provider()
            .install_default()
            .is_ok()
        {
            return Ok(());
        }

        rustls::crypto::CryptoProvider::get_default()
            .is_some_and(|provider| ring_compatible(provider))
            .then_some(())
            .ok_or(CryptoProviderConflict)
    }))
    .map_err(DockerError::CryptoProviderConflict)
}

fn ring_compatible(provider: &rustls::crypto::CryptoProvider) -> bool {
    let ring = rustls::crypto::ring::default_provider();
    std::ptr::eq(provider.secure_random, ring.secure_random)
        && std::ptr::eq(provider.key_provider, ring.key_provider)
}

#[derive(Debug)]
struct UnexpectedRedirect;

impl std::fmt::Display for UnexpectedRedirect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("unexpected redirect in response")
    }
}

impl std::error::Error for UnexpectedRedirect {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_major_minor_versions() {
        assert_eq!(parse_version("1.46").unwrap().unwrap().to_string(), "1.46");
        assert!(parse_version("").is_none());
        assert!(parse_version("1").unwrap().is_err());
        assert!(parse_version("1.2.3").unwrap().is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn raw_client_reaches_the_engine_over_a_unix_socket() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::UnixListener,
        };

        let path = env::temp_dir().join(format!(
            "ployz-internal-docker-config-{}.sock",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("POST /v1.53/images/create HTTP/1.1\r\n"));
            assert!(request.contains("host: api.moby.localhost\r\n"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let (client, base) = DaemonConfig::unix(&path).raw_client().unwrap();
        client
            .post(format!("{base}/v1.53/images/create"))
            .send()
            .await
            .unwrap();
        server.await.unwrap();
        fs::remove_file(path).unwrap();
    }
}
