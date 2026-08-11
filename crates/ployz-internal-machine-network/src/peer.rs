use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::PeerConfig;

pub(crate) struct DevicePeerSnapshot {
    pub(crate) endpoint: Option<SocketAddr>,
    pub(crate) last_handshake_time: Option<SystemTime>,
    pub(crate) receive_bytes: u64,
    pub(crate) transmit_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PeerStatus {
    Unknown,
    Up,
    Down,
}

impl PeerStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

pub(crate) const ENDPOINT_CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const PEER_DOWN_INTERVAL: Duration = Duration::from_secs(180 + 5 + 90);

pub(crate) struct Peer {
    pub(crate) config: PeerConfig,
    pub(crate) last_endpoint_change_time: Option<SystemTime>,
    pub(crate) last_handshake_time: Option<SystemTime>,
    pub(crate) receive_bytes: i64,
    pub(crate) transmit_bytes: i64,
    pub(crate) status: PeerStatus,
}

impl Peer {
    pub(crate) fn new(config: PeerConfig, device_peer: Option<&DevicePeerSnapshot>) -> Self {
        Self::new_at(config, device_peer, SystemTime::now())
    }

    fn new_at(
        config: PeerConfig,
        device_peer: Option<&DevicePeerSnapshot>,
        now: SystemTime,
    ) -> Self {
        let last_endpoint_change_time = config.endpoint.as_ref().and_then(|endpoint| {
            if device_peer.and_then(|peer| peer.endpoint) == Some(**endpoint) {
                None
            } else {
                Some(now)
            }
        });
        Self {
            config,
            last_endpoint_change_time,
            last_handshake_time: None,
            receive_bytes: 0,
            transmit_bytes: 0,
            status: PeerStatus::Unknown,
        }
    }

    pub(crate) fn update_config(&mut self, config: PeerConfig) {
        self.update_config_at(config, SystemTime::now());
    }

    fn update_config_at(&mut self, config: PeerConfig, now: SystemTime) {
        if !same_endpoint_pointer(&self.config.endpoint, &config.endpoint) {
            self.last_endpoint_change_time = Some(now);
            self.status = PeerStatus::Unknown;
        }
        self.config = config;
    }

    pub(crate) fn update_from_device(&mut self, device_peer: &DevicePeerSnapshot) -> bool {
        let mut endpoint_changed = false;
        if let Some(endpoint) = device_peer.endpoint
            && self.config.endpoint.as_deref().copied() != Some(endpoint)
        {
            self.config.endpoint = Some(Arc::new(endpoint));
            self.last_endpoint_change_time = None;
            endpoint_changed = true;
            tracing::info!(
                public_key = ?self.config.public_key,
                %endpoint,
                "Peer endpoint automatically updated on WireGuard interface by establishing a reverse connection to this machine."
            );
        }
        self.last_handshake_time = device_peer.last_handshake_time;
        self.receive_bytes = device_peer.receive_bytes as i64;
        self.transmit_bytes = device_peer.transmit_bytes as i64;
        self.calculate_status();
        endpoint_changed
    }

    pub(crate) fn calculate_status(&mut self) {
        self.calculate_status_at(SystemTime::now());
    }

    fn calculate_status_at(&mut self, now: SystemTime) {
        let last_status = self.status;
        let since_last_handshake = elapsed_since(now, self.last_handshake_time);
        let since_endpoint_change = elapsed_since(now, self.last_endpoint_change_time);
        let handshake_after_endpoint =
            is_after(self.last_handshake_time, self.last_endpoint_change_time);

        self.status = if since_endpoint_change > PEER_DOWN_INTERVAL {
            if since_last_handshake < PEER_DOWN_INTERVAL {
                PeerStatus::Up
            } else {
                PeerStatus::Down
            }
        } else if since_endpoint_change < ENDPOINT_CONNECTION_TIMEOUT {
            if handshake_after_endpoint {
                PeerStatus::Up
            } else {
                PeerStatus::Unknown
            }
        } else if handshake_after_endpoint {
            PeerStatus::Up
        } else {
            PeerStatus::Down
        };

        if self.status == PeerStatus::Down && self.config.endpoint.is_none() {
            self.status = PeerStatus::Unknown;
        }
        if self.status != last_status {
            tracing::info!(
                public_key = ?self.config.public_key,
                status = self.status.as_str(),
                previous_status = last_status.as_str(),
                "Peer status changed."
            );
        }
    }

    pub(crate) fn should_change_endpoint(&self) -> Option<SocketAddr> {
        if self.config.endpoint.is_some() && self.status != PeerStatus::Down {
            return None;
        }
        let first = self.config.all_endpoints.first()?;
        let Some(current) = &self.config.endpoint else {
            return Some(**first);
        };
        if self.config.all_endpoints.len() == 1 && Arc::ptr_eq(current, first) {
            return None;
        }
        let index = self
            .config
            .all_endpoints
            .iter()
            .position(|endpoint| **endpoint == **current);
        Some(
            *self.config.all_endpoints
                [index.map_or(0, |value| value + 1) % self.config.all_endpoints.len()],
        )
    }
}

fn same_endpoint_pointer(left: &Option<Arc<SocketAddr>>, right: &Option<Arc<SocketAddr>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn elapsed_since(now: SystemTime, instant: Option<SystemTime>) -> Duration {
    match instant {
        Some(instant) => now.duration_since(instant).unwrap_or(Duration::ZERO),
        None => Duration::MAX,
    }
}

fn is_after(left: Option<SystemTime>, right: Option<SystemTime>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left > right,
        (Some(_), None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer_with_endpoint(endpoint: Option<SocketAddr>) -> Peer {
        Peer::new_at(
            PeerConfig {
                all_endpoints: endpoint.into_iter().map(Arc::new).collect(),
                endpoint: endpoint.map(Arc::new),
                ..PeerConfig::default()
            },
            None,
            SystemTime::UNIX_EPOCH,
        )
    }

    fn device_peer(
        endpoint: Option<SocketAddr>,
        last_handshake_time: Option<SystemTime>,
    ) -> DevicePeerSnapshot {
        DevicePeerSnapshot {
            endpoint,
            last_handshake_time,
            receive_bytes: 123,
            transmit_bytes: 456,
        }
    }

    #[test]
    fn recently_changed_endpoint_stays_unknown_without_handshake() {
        let endpoint = "192.0.2.1:51820".parse().expect("valid fixture");
        let mut peer = peer_with_endpoint(Some(endpoint));
        peer.calculate_status_at(SystemTime::UNIX_EPOCH + Duration::from_secs(14));
        assert_eq!(peer.status, PeerStatus::Unknown);
    }

    #[test]
    fn endpoint_goes_down_after_initial_timeout_without_handshake() {
        let endpoint = "192.0.2.1:51820".parse().expect("valid fixture");
        let mut peer = peer_with_endpoint(Some(endpoint));
        peer.calculate_status_at(SystemTime::UNIX_EPOCH + Duration::from_secs(16));
        assert_eq!(peer.status, PeerStatus::Down);
    }

    #[test]
    fn endpoint_rotates_to_item_after_current_or_first_when_current_is_absent() {
        let first = "192.0.2.1:51820".parse().expect("valid fixture");
        let second = "192.0.2.2:51820".parse().expect("valid fixture");
        let mut peer = peer_with_endpoint(Some(first));
        peer.config.all_endpoints = vec![Arc::new(first), Arc::new(second)];
        peer.status = PeerStatus::Down;
        assert_eq!(peer.should_change_endpoint(), Some(second));
        peer.config.endpoint = Some(Arc::new("192.0.2.99:51820".parse().expect("valid fixture")));
        assert_eq!(peer.should_change_endpoint(), Some(first));
    }

    #[test]
    fn single_endpoint_guard_preserves_go_pointer_identity() {
        let endpoint = "192.0.2.1:51820".parse().expect("valid fixture");
        let shared = Arc::new(endpoint);
        let mut peer = Peer::new_at(
            PeerConfig {
                endpoint: Some(Arc::clone(&shared)),
                all_endpoints: vec![shared],
                ..PeerConfig::default()
            },
            None,
            SystemTime::UNIX_EPOCH,
        );
        peer.status = PeerStatus::Down;
        assert_eq!(peer.should_change_endpoint(), None);

        peer.config.endpoint = Some(Arc::new(endpoint));
        assert_eq!(peer.should_change_endpoint(), Some(endpoint));
    }

    #[test]
    fn zero_wireguard_handshake_remains_absent() {
        let endpoint = "192.0.2.1:51820".parse().expect("valid fixture");
        let mut peer = peer_with_endpoint(Some(endpoint));

        assert!(!peer.update_from_device(&device_peer(Some(endpoint), None)));
        assert_eq!(peer.last_handshake_time, None);
        assert_eq!(peer.receive_bytes, 123);
        assert_eq!(peer.transmit_bytes, 456);
    }
}
