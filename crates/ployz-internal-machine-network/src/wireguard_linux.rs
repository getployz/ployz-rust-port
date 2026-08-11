use std::{
    collections::HashMap,
    io,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::{Duration, SystemTime},
};

use futures_util::TryStreamExt;
use ipnet::IpNet;
use netlink_packet_route::{
    address::{AddressAttribute, AddressMessage},
    link::{LinkAttribute, LinkFlags, LinkMessage},
    route::{
        RouteAddress, RouteAttribute, RouteFlags, RouteHeader, RouteMessage, RouteMetric,
        RouteScope,
    },
};
use rtnetlink::{Handle, LinkUnspec, LinkWireguard, RouteMessageBuilder};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use wireguard_uapi::{
    DeviceInterface, WgSocket,
    get::{Device as WireGuardDevice, Peer as WireGuardPeer},
    set::{
        AllowedIp as WireGuardAllowedIp, Device as WireGuardDeviceUpdate,
        Peer as WireGuardPeerUpdate, WgPeerF,
    },
};

use crate::{
    Config, EndpointChangeEvent, EndpointWatcher, NetworkError, WIREGUARD_INTERFACE_NAME,
    WIREGUARD_KEEPALIVE_INTERVAL, machine_ip,
    peer::{DevicePeerSnapshot, Peer},
    wireguard::{EndpointSender, endpoint_envelope, key_from_secret, secret_from_bytes},
};

struct State {
    link_index: u32,
    peers: Option<HashMap<String, Peer>>,
    watchers: Vec<EndpointSender>,
    running: bool,
}

/// Linux kernel WireGuard interface and its endpoint-management control loop.
pub struct WireGuardNetwork {
    state: Mutex<State>,
}

impl WireGuardNetwork {
    /// Creates the `ployz` WireGuard link if it does not already exist.
    pub async fn new() -> Result<Self, NetworkError> {
        let handle = netlink_handle()?;
        let link = create_or_get_link(&handle, WIREGUARD_INTERFACE_NAME).await?;
        Ok(Self {
            state: Mutex::new(State {
                link_index: link.header.index,
                peers: None,
                watchers: Vec::new(),
                running: false,
            }),
        })
    }

    /// Applies device, address, MTU, link-state, and peer-route configuration.
    pub async fn configure(&self, config: Config) -> Result<(), NetworkError> {
        let mut state = self.state.lock().await;
        self.configure_device(&mut state, &config)?;
        tracing::info!(
            name = WIREGUARD_INTERFACE_NAME,
            peers = state.peers.as_ref().map_or(0, HashMap::len),
            "Configured WireGuard interface."
        );

        let management_ip = config.management_ip.ok_or_else(|| {
            NetworkError::Invalid("parse management IP: invalid IP address".into())
        })?;
        let addresses = vec![crate::ip::single_ip_prefix(management_ip)?];
        let handle = netlink_handle()?;
        update_addresses(&handle, state.link_index, &addresses).await?;
        tracing::info!(
            name = WIREGUARD_INTERFACE_NAME,
            ?addresses,
            "Updated addresses of the WireGuard interface."
        );

        let mtu = u32::try_from(config.effective_mtu()).map_err(|_| {
            NetworkError::Invalid(format!(
                "set MTU {} on WireGuard link: invalid MTU",
                config.effective_mtu()
            ))
        })?;
        let link = get_link_by_index(&handle, state.link_index).await?;
        if link_mtu(&link) != Some(mtu) {
            handle
                .link()
                .change(
                    LinkUnspec::new_with_index(state.link_index)
                        .mtu(mtu)
                        .build(),
                )
                .execute()
                .await
                .map_err(|source| {
                    netlink_error(
                        format!("set MTU {mtu} on WireGuard link {WIREGUARD_INTERFACE_NAME:?}"),
                        source,
                    )
                })?;
        }
        if !link.header.flags.contains(LinkFlags::Up) {
            handle
                .link()
                .change(LinkUnspec::new_with_index(state.link_index).up().build())
                .execute()
                .await
                .map_err(|source| {
                    netlink_error(
                        format!("set WireGuard link {WIREGUARD_INTERFACE_NAME:?} up"),
                        source,
                    )
                })?;
        }

        let subnet = config
            .subnet
            .ok_or_else(|| NetworkError::Invalid("machine subnet is absent".into()))?;
        update_peer_routes(
            &handle,
            state.link_index,
            machine_ip(subnet).ok_or_else(|| {
                NetworkError::Invalid("machine subnet has no next IP address".into())
            })?,
            state
                .peers
                .as_ref()
                .ok_or_else(|| NetworkError::Invalid("peers are not configured".into()))?,
        )
        .await?;
        Ok(())
    }

    fn configure_device(&self, state: &mut State, config: &Config) -> Result<(), NetworkError> {
        let mut client = wireguard_client()?;
        let device = get_wireguard_device(&mut client)?;

        if state.peers.is_none() {
            let device_peers: HashMap<String, _> = device
                .peers
                .iter()
                .map(|peer| (key_hex(&peer.public_key), device_peer_snapshot(peer)))
                .collect();
            let mut peers = HashMap::with_capacity(config.peers.len());
            for peer_config in &config.peers {
                let key = peer_config.key_string()?;
                peers.insert(
                    key.clone(),
                    Peer::new(peer_config.clone(), device_peers.get(&key)),
                );
            }
            state.peers = Some(peers);
        }

        let peers = state
            .peers
            .as_mut()
            .ok_or_else(|| NetworkError::Invalid("peers are not configured".into()))?;
        let mut new_peer_keys = Vec::with_capacity(config.peers.len());
        for peer_config in &config.peers {
            let key = peer_config.key_string()?;
            if let Some(peer) = peers.get_mut(&key) {
                peer.update_config(peer_config.clone());
            } else {
                peers.insert(key.clone(), Peer::new(peer_config.clone(), None));
            }
            new_peer_keys.push(key);
        }
        peers.retain(|key, _| new_peer_keys.contains(key));

        apply_wireguard_config(&mut client, config, &device.peers)
    }

    /// Runs one-second peer status and endpoint-rotation ticks until cancelled.
    pub async fn run(&self, cancellation: CancellationToken) -> Result<(), NetworkError> {
        wireguard_client()?;
        {
            let mut state = self.state.lock().await;
            if state.running {
                return Err(NetworkError::Invalid("network is already running".into()));
            }
            state.running = true;
        }

        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let mut state = self.state.lock().await;
                    if let Err(error) = self.update_peers_from_device(&mut state, &cancellation).await {
                        tracing::error!(name = WIREGUARD_INTERFACE_NAME, %error, "Failed to update peers status from WireGuard interface.");
                    }
                    if let Err(error) = self.change_wire_guard_endpoints(&mut state, &cancellation).await {
                        tracing::error!(name = WIREGUARD_INTERFACE_NAME, %error, "Failed to update peer endpoints on WireGuard interface.");
                    }
                }
                () = cancellation.cancelled() => {
                    let mut state = self.state.lock().await;
                    state.watchers.clear();
                    state.running = false;
                    return Ok(());
                }
            }
        }
    }

    /// Registers an unbuffered-equivalent endpoint event stream.
    pub async fn watch_endpoints(&self) -> EndpointWatcher {
        let (sender, receiver) = EndpointWatcher::channel();
        self.state.lock().await.watchers.push(sender);
        receiver
    }

    async fn update_peers_from_device(
        &self,
        state: &mut State,
        cancellation: &CancellationToken,
    ) -> Result<(), NetworkError> {
        let mut client = wireguard_client()?;
        let device = get_wireguard_device(&mut client)?;
        let peers = state
            .peers
            .as_mut()
            .ok_or_else(|| NetworkError::Invalid("peers are not configured".into()))?;
        let mut events = Vec::new();
        for device_peer in &device.peers {
            let key = key_hex(&device_peer.public_key);
            if let Some(peer) = peers.get_mut(&key) {
                if peer.update_from_device(&device_peer_snapshot(device_peer)) {
                    let endpoint = peer.config.endpoint.as_deref().copied().ok_or_else(|| {
                        NetworkError::Invalid("updated peer endpoint is absent".into())
                    })?;
                    events.push(EndpointChangeEvent {
                        public_key: secret_from_bytes(&device_peer.public_key)?,
                        endpoint,
                    });
                }
            } else {
                tracing::warn!(
                    public_key = key,
                    "Found WireGuard peer that is not in the configuration."
                );
            }
        }
        if !events.is_empty()
            && let Err(error) = notify_watchers(&state.watchers, &events, cancellation).await
        {
            tracing::error!(%error, "Failed to notify watchers about a peer endpoint change.");
        }
        Ok(())
    }

    async fn change_wire_guard_endpoints(
        &self,
        state: &mut State,
        cancellation: &CancellationToken,
    ) -> Result<(), NetworkError> {
        let peers = state
            .peers
            .as_mut()
            .ok_or_else(|| NetworkError::Invalid("peers are not configured".into()))?;
        let mut events = Vec::new();
        for peer in peers.values_mut() {
            let Some(endpoint) = peer.should_change_endpoint() else {
                continue;
            };
            let mut config = peer.config.clone();
            config.endpoint = Some(Arc::new(endpoint));
            peer.update_config(config);
            let public_secret = peer
                .config
                .public_key
                .as_ref()
                .ok_or_else(|| NetworkError::Invalid("peer public key is absent".into()))?;
            events.push(EndpointChangeEvent {
                public_key: public_secret.clone(),
                endpoint,
            });
        }
        if events.is_empty() {
            return Ok(());
        }
        let mut client = wireguard_client()?;
        let device = get_wireguard_device(&mut client)?;
        let changed_keys = events
            .iter()
            .map(|event| key_from_secret(&event.public_key, "peer public key"))
            .collect::<Result<Vec<_>, NetworkError>>()?;
        if let Some(missing_index) = changed_keys.iter().position(|key| {
            !device
                .peers
                .iter()
                .any(|peer| peer.public_key.as_slice() == key.as_slice())
        }) {
            let missing = &events[missing_index];
            return Err(NetworkError::Invalid(format!(
                "configure WireGuard device {WIREGUARD_INTERFACE_NAME:?} with endpoint changes: update-only peer {} does not exist",
                missing.public_key.to_hex_string()
            )));
        }
        let peer_updates = events
            .iter()
            .zip(changed_keys)
            .map(|(event, public_key)| {
                WireGuardPeerUpdate::from_public_key(public_key)
                    .flags(vec![WgPeerF::UpdateOnly])
                    .endpoint(&event.endpoint)
            })
            .collect();
        client
            .set_device(
                WireGuardDeviceUpdate::from_ifname(WIREGUARD_INTERFACE_NAME).peers(peer_updates),
            )
            .map_err(|source| {
                wireguard_error(
                    format!(
                        "configure WireGuard device {WIREGUARD_INTERFACE_NAME:?} with endpoint changes"
                    ),
                    source,
                )
            })?;
        for event in &events {
            if let Some(peer) = peers.get(&event.public_key.to_hex_string()) {
                tracing::info!(name = WIREGUARD_INTERFACE_NAME, public_key = %event.public_key, endpoint = %event.endpoint, status = peer.status.as_str(), "Changed peer endpoint on WireGuard interface.");
            }
        }
        if let Err(error) = notify_watchers(&state.watchers, &events, cancellation).await {
            tracing::error!(%error, "Failed to notify watchers about a peer endpoint change.");
        }
        Ok(())
    }

    /// Deletes the WireGuard link when the control loop is stopped.
    pub async fn cleanup(&self) -> Result<(), NetworkError> {
        let state = self.state.lock().await;
        if state.running {
            return Err(NetworkError::Invalid(
                "network is still running, stop it before cleanup".into(),
            ));
        }
        let handle = netlink_handle()?;
        handle
            .link()
            .del(state.link_index)
            .execute()
            .await
            .map_err(|source| {
                netlink_error(
                    format!("delete WireGuard link {WIREGUARD_INTERFACE_NAME:?}"),
                    source,
                )
            })?;
        tracing::info!(
            name = WIREGUARD_INTERFACE_NAME,
            "Deleted WireGuard interface."
        );
        Ok(())
    }
}

pub(crate) async fn detect_egress_mtu() -> Result<u32, NetworkError> {
    let handle = netlink_handle()?;
    let route = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::new(1, 1, 1, 1), 32)
        .build();
    let mut routes = handle.route().get(route).execute();
    let route = routes
        .try_next()
        .await
        .map_err(|source| netlink_error("get route to public address", source))?
        .ok_or_else(|| NetworkError::Invalid("no route to public address".into()))?;
    let link_index = route
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            RouteAttribute::Oif(index) => Some(*index),
            _ => None,
        })
        .ok_or_else(|| {
            NetworkError::Invalid(
                "get route to public address: route has no egress interface".into(),
            )
        })?;
    let link = get_link_by_index(&handle, link_index).await?;
    if link_name(&link).as_deref() == Some(WIREGUARD_INTERFACE_NAME) {
        return Err(NetworkError::Invalid(format!(
            "egress interface is the WireGuard interface '{WIREGUARD_INTERFACE_NAME}'"
        )));
    }
    if let Some(mtu) = route
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            RouteAttribute::Metrics(metrics) => metrics.iter().find_map(|metric| match metric {
                RouteMetric::Mtu(mtu) if *mtu > 0 => Some(*mtu),
                _ => None,
            }),
            _ => None,
        })
    {
        return Ok(mtu);
    }
    link_mtu(&link).ok_or_else(|| {
        NetworkError::Invalid("get egress interface for route: interface has no MTU".into())
    })
}

async fn create_or_get_link(handle: &Handle, name: &str) -> Result<LinkMessage, NetworkError> {
    if let Some(link) = find_link_by_name(handle, name).await? {
        tracing::info!(name, "Found existing WireGuard interface.");
        return Ok(link);
    }
    handle
        .link()
        .add(LinkWireguard::new(name).build())
        .execute()
        .await
        .map_err(|source| netlink_error(format!("create WireGuard link {name:?}"), source))?;
    tracing::info!(name, "Created WireGuard interface.");
    find_link_by_name(handle, name).await?.ok_or_else(|| {
        NetworkError::Invalid(format!(
            "find created WireGuard link {name:?}: link disappeared"
        ))
    })
}

async fn find_link_by_name(
    handle: &Handle,
    name: &str,
) -> Result<Option<LinkMessage>, NetworkError> {
    handle
        .link()
        .get()
        .match_name(name.to_owned())
        .execute()
        .try_next()
        .await
        .map_err(|source| netlink_error(format!("find WireGuard link {name:?}"), source))
}

async fn get_link_by_index(handle: &Handle, index: u32) -> Result<LinkMessage, NetworkError> {
    handle
        .link()
        .get()
        .match_index(index)
        .execute()
        .try_next()
        .await
        .map_err(|source| netlink_error(format!("get network interface index {index}"), source))?
        .ok_or_else(|| {
            NetworkError::Invalid(format!("get network interface index {index}: not found"))
        })
}

fn link_name(link: &LinkMessage) -> Option<String> {
    link.attributes
        .iter()
        .find_map(|attribute| match attribute {
            LinkAttribute::IfName(name) => Some(name.clone()),
            _ => None,
        })
}

fn link_mtu(link: &LinkMessage) -> Option<u32> {
    link.attributes
        .iter()
        .find_map(|attribute| match attribute {
            LinkAttribute::Mtu(mtu) => Some(*mtu),
            _ => None,
        })
}

async fn update_addresses(
    handle: &Handle,
    index: u32,
    addresses: &[IpNet],
) -> Result<(), NetworkError> {
    for address in addresses {
        let result = handle
            .address()
            .add(index, address.addr(), address.prefix_len())
            .execute()
            .await;
        if let Err(source) = result
            && !is_errno(&source, libc::EEXIST)
        {
            return Err(netlink_error(
                format!("add subnet address to WireGuard link {WIREGUARD_INTERFACE_NAME:?}"),
                source,
            ));
        }
    }
    let mut current = handle
        .address()
        .get()
        .set_link_index_filter(index)
        .execute();
    while let Some(message) = current.try_next().await.map_err(|source| {
        netlink_error(
            format!("list addresses on WireGuard link {WIREGUARD_INTERFACE_NAME:?}"),
            source,
        )
    })? {
        let prefix = address_message_prefix(&message)?;
        if addresses.contains(&prefix) {
            continue;
        }
        handle
            .address()
            .del(message)
            .execute()
            .await
            .map_err(|source| {
                netlink_error(
                    format!(
                        "remove address {prefix:?} from WireGuard link {WIREGUARD_INTERFACE_NAME:?}"
                    ),
                    source,
                )
            })?;
    }
    Ok(())
}

fn address_message_prefix(message: &AddressMessage) -> Result<IpNet, NetworkError> {
    let address = message
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            AddressAttribute::Local(address) | AddressAttribute::Address(address) => Some(*address),
            _ => None,
        })
        .ok_or_else(|| NetworkError::Invalid("invalid IP network: address is absent".into()))?;
    IpNet::new(address, message.header.prefix_len)
        .map(|prefix| prefix.trunc())
        .map_err(|error| NetworkError::Invalid(format!("invalid IP network: {error}")))
}

async fn update_peer_routes(
    handle: &Handle,
    link_index: u32,
    machine_ip: IpAddr,
    peers: &HashMap<String, Peer>,
) -> Result<(), NetworkError> {
    let mut prefixes = Vec::new();
    for peer in peers.values() {
        prefixes.extend(peer.config.prefixes()?);
    }
    let prefixes = compact_prefixes(prefixes);
    for prefix in &prefixes {
        let route = match (*prefix, machine_ip) {
            (IpNet::V4(prefix), IpAddr::V4(source)) => RouteMessageBuilder::<Ipv4Addr>::new()
                .destination_prefix(prefix.network(), prefix.prefix_len())
                .pref_source(source)
                .output_interface(link_index)
                .scope(RouteScope::Link)
                .build(),
            (IpNet::V6(prefix), _) => RouteMessageBuilder::<std::net::Ipv6Addr>::new()
                .destination_prefix(prefix.network(), prefix.prefix_len())
                .output_interface(link_index)
                .scope(RouteScope::Link)
                .build(),
            (IpNet::V4(_), IpAddr::V6(_)) => {
                return Err(NetworkError::Invalid(
                    "IPv4 peer route requires an IPv4 machine IP".into(),
                ));
            }
        };
        if let Err(source) = handle.route().add(route).execute().await
            && !is_errno(&source, libc::EEXIST)
        {
            return Err(netlink_error(
                format!("add route to WireGuard link {WIREGUARD_INTERFACE_NAME:?}"),
                source,
            ));
        }
    }

    for query in [
        RouteMessageBuilder::<Ipv4Addr>::new().build(),
        RouteMessageBuilder::<std::net::Ipv6Addr>::new().build(),
    ] {
        let mut routes = handle.route().get(query).execute();
        while let Some(route) = routes.try_next().await.map_err(|source| {
            netlink_error(
                format!("list routes on WireGuard link {WIREGUARD_INTERFACE_NAME:?}"),
                source,
            )
        })? {
            let table = route
                .attributes
                .iter()
                .find_map(|attribute| match attribute {
                    RouteAttribute::Table(table) => Some(*table),
                    _ => None,
                });
            if table.unwrap_or(u32::from(route.header.table))
                != u32::from(RouteHeader::RT_TABLE_MAIN)
                || route.header.flags.contains(RouteFlags::Cloned)
            {
                continue;
            }
            if route_output_interface(&route) != Some(link_index) {
                continue;
            }
            let route_prefix = route_message_prefix(&route)?;
            if prefixes.contains(&route_prefix) {
                continue;
            }
            handle.route().del(route).execute().await.map_err(|source| netlink_error(format!("remove route {route_prefix} from WireGuard link {WIREGUARD_INTERFACE_NAME:?}"), source))?;
        }
    }
    Ok(())
}

fn route_output_interface(route: &RouteMessage) -> Option<u32> {
    route
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            RouteAttribute::Oif(index) => Some(*index),
            _ => None,
        })
}

fn route_message_prefix(route: &RouteMessage) -> Result<IpNet, NetworkError> {
    let address = route
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            RouteAttribute::Destination(RouteAddress::Inet(address)) => Some(IpAddr::V4(*address)),
            RouteAttribute::Destination(RouteAddress::Inet6(address)) => Some(IpAddr::V6(*address)),
            _ => None,
        })
        .ok_or_else(|| {
            NetworkError::Invalid("parse route destination: invalid IP network".into())
        })?;
    IpNet::new(address, route.header.destination_prefix_length)
        .map(|prefix| prefix.trunc())
        .map_err(|error| NetworkError::Invalid(format!("parse route destination: {error}")))
}

fn compact_prefixes(mut prefixes: Vec<IpNet>) -> Vec<IpNet> {
    prefixes = prefixes.into_iter().map(|prefix| prefix.trunc()).collect();
    loop {
        prefixes.sort_by_key(|prefix| (prefix.addr(), prefix.prefix_len()));
        prefixes.dedup();
        let mut changed = false;
        let snapshot = prefixes.clone();
        prefixes.retain(|prefix| {
            let contained = snapshot
                .iter()
                .any(|other| other != prefix && other.contains(prefix));
            changed |= contained;
            !contained
        });
        if changed {
            continue;
        }
        let mut pair = None;
        'outer: for left in 0..prefixes.len() {
            for right in left + 1..prefixes.len() {
                if prefixes[left].is_sibling(&prefixes[right]) {
                    pair = Some((left, right));
                    break 'outer;
                }
            }
        }
        let Some((left, right)) = pair else {
            return prefixes;
        };
        let supernet = prefixes[left].supernet();
        prefixes.remove(right);
        prefixes.remove(left);
        if let Some(supernet) = supernet {
            prefixes.push(supernet);
        }
    }
}

async fn notify_watchers(
    watchers: &[EndpointSender],
    events: &[EndpointChangeEvent],
    cancellation: &CancellationToken,
) -> Result<(), NetworkError> {
    for watcher in watchers {
        for event in events {
            let delivery = async {
                let (envelope, received) = endpoint_envelope(event.clone());
                watcher
                    .send(envelope)
                    .await
                    .map_err(|_| NetworkError::Invalid("endpoint watcher is closed".into()))?;
                received
                    .await
                    .map_err(|_| NetworkError::Invalid("endpoint watcher is closed".into()))
            };
            tokio::select! {
                result = delivery => result?,
                () = tokio::time::sleep(Duration::from_secs(1)) => {
                    return Err(NetworkError::Invalid("timeout 1 second".into()));
                }
                () = cancellation.cancelled() => return Ok(()),
            }
        }
    }
    Ok(())
}

fn apply_wireguard_config(
    client: &mut WgSocket,
    config: &Config,
    current_peers: &[WireGuardPeer],
) -> Result<(), NetworkError> {
    let private_key = config
        .private_key
        .as_ref()
        .ok_or_else(|| NetworkError::Invalid("parse private key: key is absent".into()))?;
    let private_key = key_from_secret(private_key, "private key")?;
    let listen_port = u16::try_from(config.effective_wire_guard_port()).map_err(|_| {
        NetworkError::Invalid(format!(
            "parse WireGuard listen port: invalid port {}",
            config.effective_wire_guard_port()
        ))
    })?;

    let allowed_ip_values = config
        .peers
        .iter()
        .map(|peer| {
            peer.prefixes().map(|prefixes| {
                prefixes
                    .into_iter()
                    .map(|prefix| (prefix.addr(), prefix.prefix_len()))
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Result<Vec<_>, NetworkError>>()?;

    let mut peer_updates = Vec::with_capacity(config.peers.len() + current_peers.len());
    for (peer, allowed_ips) in config.peers.iter().zip(&allowed_ip_values) {
        let public_key = peer
            .public_key
            .as_ref()
            .ok_or_else(|| NetworkError::Invalid("parse peer public key: key is absent".into()))?;
        let public_key = key_from_secret(public_key, "peer public key")?;
        let allowed_ips = allowed_ips
            .iter()
            .map(|(address, cidr)| {
                let mut allowed_ip = WireGuardAllowedIp::from_ipaddr(address);
                allowed_ip.cidr_mask = Some(*cidr);
                allowed_ip
            })
            .collect();
        let mut update = WireGuardPeerUpdate::from_public_key(public_key)
            .flags(vec![WgPeerF::ReplaceAllowedIps])
            .persistent_keepalive_interval(WIREGUARD_KEEPALIVE_INTERVAL.as_secs() as u16)
            .allowed_ips(allowed_ips);
        if let Some(endpoint) = peer.endpoint.as_deref() {
            update = update.endpoint(endpoint);
        }
        peer_updates.push(update);
    }

    let configured_peer_count = peer_updates.len();
    for current_peer in current_peers {
        if !peer_updates[..configured_peer_count]
            .iter()
            .any(|peer| peer.public_key == &current_peer.public_key)
        {
            peer_updates.push(
                WireGuardPeerUpdate::from_public_key(&current_peer.public_key)
                    .flags(vec![WgPeerF::RemoveMe]),
            );
        }
    }

    client
        .set_device(
            WireGuardDeviceUpdate::from_ifname(WIREGUARD_INTERFACE_NAME)
                .private_key(private_key)
                .listen_port(listen_port)
                .peers(peer_updates),
        )
        .map_err(|source| {
            wireguard_error(
                format!("configure WireGuard device {WIREGUARD_INTERFACE_NAME:?}"),
                source,
            )
        })
}

fn wireguard_client() -> Result<WgSocket, NetworkError> {
    WgSocket::connect().map_err(|source| wireguard_error("create WireGuard client", source))
}

fn get_wireguard_device(client: &mut WgSocket) -> Result<WireGuardDevice, NetworkError> {
    client
        .get_device(DeviceInterface::from_name(WIREGUARD_INTERFACE_NAME))
        .map_err(|source| {
            wireguard_error(
                format!("get WireGuard device {WIREGUARD_INTERFACE_NAME:?}"),
                source,
            )
        })
}

fn device_peer_snapshot(peer: &WireGuardPeer) -> DevicePeerSnapshot {
    let last_handshake_time = if peer.last_handshake_time == Duration::ZERO {
        None
    } else {
        SystemTime::UNIX_EPOCH.checked_add(peer.last_handshake_time)
    };
    DevicePeerSnapshot {
        endpoint: peer.endpoint,
        last_handshake_time,
        receive_bytes: peer.rx_bytes,
        transmit_bytes: peer.tx_bytes,
    }
}

fn wireguard_error(
    context: impl Into<String>,
    source: impl std::error::Error + Send + Sync + 'static,
) -> NetworkError {
    crate::io_error(context, io::Error::other(source))
}

fn netlink_handle() -> Result<Handle, NetworkError> {
    let (connection, handle, _) = rtnetlink::new_connection()
        .map_err(|source| crate::io_error("create route netlink client", source))?;
    tokio::spawn(connection);
    Ok(handle)
}

fn key_hex(key: &[u8; 32]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_errno(error: &rtnetlink::Error, errno: i32) -> bool {
    matches!(error, rtnetlink::Error::NetlinkError(message) if message.raw_code().abs() == errno)
}

fn netlink_error(context: impl Into<String>, source: rtnetlink::Error) -> NetworkError {
    NetworkError::Netlink {
        context: context.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_compaction_merges_siblings_and_removes_contained_ranges() {
        let compacted = compact_prefixes(vec![
            "10.0.0.0/25".parse().expect("valid fixture"),
            "10.0.0.128/25".parse().expect("valid fixture"),
            "10.0.0.42/32".parse().expect("valid fixture"),
            "fdcc::1/128".parse().expect("valid fixture"),
        ]);
        assert_eq!(
            compacted,
            vec![
                "10.0.0.0/24".parse().expect("valid fixture"),
                "fdcc::1/128".parse().expect("valid fixture"),
            ]
        );
    }
}
