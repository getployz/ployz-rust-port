use std::time::Duration;

use ployz_internal_secret::Secret;
use tokio::sync::{mpsc, oneshot};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::NetworkError;

/// Name of Ployz's kernel WireGuard interface.
pub const WIREGUARD_INTERFACE_NAME: &str = "ployz";
/// Default UDP port listened on by WireGuard.
pub const DEFAULT_WIREGUARD_PORT: u16 = 51_820;
/// Minimum tunnel MTU, preserving IPv6's minimum link MTU.
pub const MIN_WIREGUARD_MTU: u32 = 1_280;
/// Worst-case outer IPv6, UDP, and WireGuard encapsulation overhead.
pub(crate) const WIREGUARD_ENCAP_OVERHEAD: u32 = 80;
/// Conservative maximum tunnel MTU for a 1500-byte underlay.
pub const MAX_WIREGUARD_MTU: u32 = 1_500 - WIREGUARD_ENCAP_OVERHEAD;
/// Persistent keepalive interval used for every peer.
pub const WIREGUARD_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(25);

/// A peer endpoint change observed or selected by the control loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointChangeEvent {
    pub public_key: Secret,
    pub endpoint: std::net::SocketAddr,
}

pub(crate) struct EndpointEnvelope {
    event: EndpointChangeEvent,
    received: oneshot::Sender<()>,
}

pub(crate) type EndpointSender = mpsc::Sender<EndpointEnvelope>;

/// Receive side of the unbuffered endpoint-change notification channel.
pub struct EndpointWatcher {
    receiver: Option<mpsc::Receiver<EndpointEnvelope>>,
}

impl EndpointWatcher {
    pub(crate) fn channel() -> (EndpointSender, Self) {
        let (sender, receiver) = mpsc::channel(1);
        (
            sender,
            Self {
                receiver: Some(receiver),
            },
        )
    }

    #[cfg(target_os = "macos")]
    pub(crate) const fn never() -> Self {
        Self { receiver: None }
    }

    /// Waits for the next endpoint change, or forever for Darwin's nil-channel stub.
    pub async fn recv(&mut self) -> Option<EndpointChangeEvent> {
        let Some(receiver) = &mut self.receiver else {
            return std::future::pending().await;
        };
        let envelope = receiver.recv().await?;
        let _ = envelope.received.send(());
        Some(envelope.event)
    }
}

pub(crate) fn endpoint_envelope(
    event: EndpointChangeEvent,
) -> (EndpointEnvelope, oneshot::Receiver<()>) {
    let (received, acknowledged) = oneshot::channel();
    (EndpointEnvelope { event, received }, acknowledged)
}

/// Generates a WireGuard private/public key pair.
pub fn new_machine_keys() -> Result<(Secret, Secret), NetworkError> {
    let generated = ployz_internal_secret::new(32).map_err(|error| {
        NetworkError::Invalid(format!("generate WireGuard private key: {error}"))
    })?;
    let mut private_bytes: [u8; 32] = generated.as_bytes().try_into().map_err(|_| {
        NetworkError::Invalid("generate WireGuard private key: wrong random key length".into())
    })?;
    private_bytes[0] &= 248;
    private_bytes[31] &= 127;
    private_bytes[31] |= 64;
    let private = StaticSecret::from(private_bytes);
    let public = PublicKey::from(&private);
    Ok((
        secret_from_bytes(&private.to_bytes())?,
        secret_from_bytes(&public.to_bytes())?,
    ))
}

pub(crate) fn key_from_secret<'a>(
    secret: &'a Secret,
    name: &str,
) -> Result<&'a [u8; 32], NetworkError> {
    secret
        .as_bytes()
        .try_into()
        .map_err(|_| NetworkError::Invalid(format!("parse {name}: WireGuard key must be 32 bytes")))
}

pub(crate) fn secret_from_bytes(bytes: &[u8; 32]) -> Result<Secret, NetworkError> {
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}")
            .map_err(|error| NetworkError::Invalid(format!("encode WireGuard key: {error}")))?;
    }
    Secret::from_hex_string(&encoded)
        .map_err(|error| NetworkError::Invalid(format!("encode WireGuard key: {error}")))
}
