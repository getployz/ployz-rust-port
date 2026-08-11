//! Rootless, in-process WireGuard transport backed by a userspace TCP/IP stack.
//!
//! The current production caller dials numeric TCP endpoints. The broader Go
//! netstack surface (UDP, ping, and in-tunnel DNS names) is deliberately
//! rejected with a typed error instead of being emulated incompletely.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    fmt,
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, UdpSocket},
    str::FromStr,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    task::{Context as TaskContext, Poll, Waker},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use boringtun::{
    noise::{Tunn, TunnResult, errors::WireGuardError},
    x25519::{PublicKey, StaticSecret},
};
use ployz_internal_secret::Secret;
use smoltcp::{
    iface::{Config as InterfaceConfig, Interface, Route, SocketHandle, SocketSet},
    phy::{self, Device, DeviceCapabilities, Medium},
    socket::tcp,
    time::Instant as SmolInstant,
    wire::{HardwareAddress, IpCidr, IpEndpoint},
};

/// WireGuard's conventional UDP endpoint port.
pub const DEFAULT_ENDPOINT_PORT: u16 = 51_820;
/// The keepalive interval used when no interval is configured.
pub const DEFAULT_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(25);
/// WireGuard's default layer-three MTU.
pub const DEFAULT_MTU: usize = 1_420;

const TCP_BUFFER_SIZE: usize = 64 * 1024;
const MAX_UDP_DATAGRAM: usize = u16::MAX as usize + 1;
const DRIVER_TICK: Duration = Duration::from_millis(10);
const TIMER_TICK: Duration = Duration::from_millis(250);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_COMMANDS: usize = 256;
const MAX_STREAMS: usize = 256;
const MAX_PENDING_DATAGRAMS: usize = 256;
const MAX_PLAINTEXT_PACKETS: usize = 256;
const MAX_DATAGRAMS_PER_TICK: usize = 64;

/// An IP network prefix routed to the WireGuard peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpPrefix {
    address: IpAddr,
    prefix_len: u8,
}

impl IpPrefix {
    /// Creates a validated IPv4 or IPv6 network prefix.
    pub fn new(address: IpAddr, prefix_len: u8) -> Result<Self, TunnelError> {
        let maximum = if address.is_ipv4() { 32 } else { 128 };
        if prefix_len > maximum {
            return Err(TunnelError::configure(format!(
                "invalid prefix length {prefix_len} for {address}"
            )));
        }
        Ok(Self {
            address,
            prefix_len,
        })
    }

    /// Returns the prefix's address.
    #[must_use]
    pub const fn address(self) -> IpAddr {
        self.address
    }

    /// Returns the number of network bits.
    #[must_use]
    pub const fn prefix_len(self) -> u8 {
        self.prefix_len
    }

    /// Reports whether an address is covered by this prefix.
    #[must_use]
    pub fn contains(self, candidate: IpAddr) -> bool {
        if self.prefix_len == 0 {
            return self.address.is_ipv4() == candidate.is_ipv4();
        }
        match (self.address, candidate) {
            (IpAddr::V4(network), IpAddr::V4(candidate)) => {
                let shift = 32 - u32::from(self.prefix_len);
                (u32::from(network) >> shift) == (u32::from(candidate) >> shift)
            }
            (IpAddr::V6(network), IpAddr::V6(candidate)) => {
                let shift = 128 - u32::from(self.prefix_len);
                (u128::from(network) >> shift) == (u128::from(candidate) >> shift)
            }
            _ => false,
        }
    }

    fn as_cidr(self) -> IpCidr {
        IpCidr::new(self.address.into(), self.prefix_len)
    }
}

/// Construction settings for one point-to-point WireGuard tunnel.
#[derive(Clone)]
pub struct TunnelConfig {
    pub local_address: IpAddr,
    pub local_private_key: Secret,
    pub endpoint: SocketAddr,
    pub remote_public_key: Secret,
    pub remote_network: IpPrefix,
    pub dns: Option<IpAddr>,
    pub mtu: usize,
    pub keep_alive: Duration,
}

impl fmt::Debug for TunnelConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TunnelConfig")
            .field("local_address", &self.local_address)
            .field("local_private_key", &"[REDACTED]")
            .field("endpoint", &self.endpoint)
            .field("remote_public_key", &"[REDACTED]")
            .field("remote_network", &self.remote_network)
            .field("dns", &self.dns)
            .field("mtu", &self.mtu)
            .field("keep_alive", &self.keep_alive)
            .finish()
    }
}

/// A clonable cancellation signal for an in-progress dial.
#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    /// Cancels operations observing this signal.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Reports whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Context for cancellation and optional dial deadline.
#[derive(Clone, Debug, Default)]
pub struct DialContext {
    cancellation: Cancellation,
    deadline: Option<Instant>,
}

impl DialContext {
    /// Creates a context that remains active until explicitly cancelled.
    #[must_use]
    pub fn background() -> Self {
        Self::default()
    }

    /// Creates a context with a relative deadline.
    #[must_use]
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            cancellation: Cancellation::default(),
            deadline: Instant::now().checked_add(timeout),
        }
    }

    /// Returns the cancellation signal owned by this context.
    #[must_use]
    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }

    /// Cancels this context.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    fn remaining(&self) -> Result<Option<Duration>, TunnelError> {
        if self.cancellation.is_cancelled() {
            return Err(TunnelError::dial("operation cancelled"));
        }
        self.deadline.map_or(Ok(None), |deadline| {
            deadline
                .checked_duration_since(Instant::now())
                .map(Some)
                .ok_or_else(|| TunnelError::dial("deadline exceeded"))
        })
    }
}

/// A contextual tunnel construction or I/O failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelError {
    context: &'static str,
    detail: String,
}

impl TunnelError {
    fn create(detail: impl Into<String>) -> Self {
        Self::new("create WireGuard TUN device", detail)
    }

    fn configure(detail: impl Into<String>) -> Self {
        Self::new("configure WireGuard device", detail)
    }

    fn enable(detail: impl Into<String>) -> Self {
        Self::new("enable WireGuard device", detail)
    }

    fn dial(detail: impl Into<String>) -> Self {
        Self::new("dial through WireGuard tunnel", detail)
    }

    fn new(context: &'static str, detail: impl Into<String>) -> Self {
        Self {
            context,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for TunnelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.context, self.detail)
    }
}

impl Error for TunnelError {}

/// An active userspace WireGuard device and TCP/IP stack.
pub struct Tunnel {
    sender: SyncSender<Command>,
    health: Arc<Health>,
    driver: Mutex<Option<JoinHandle<()>>>,
}

impl Tunnel {
    /// Configures and activates a rootless userspace WireGuard tunnel.
    pub fn connect(mut config: TunnelConfig) -> Result<Self, TunnelError> {
        apply_defaults(&mut config);
        let validated = ValidatedConfig::try_from(config)?;
        let udp = create_udp_socket(validated.endpoint)?;
        let health = Arc::new(Health::default());
        let (sender, receiver) = mpsc::sync_channel(MAX_COMMANDS);
        let driver_health = health.clone();
        let driver = thread::Builder::new()
            .name("ployz-wireguard".into())
            .spawn(move || Driver::new(validated, udp, receiver, driver_health).run())
            .map_err(|error| TunnelError::enable(error.to_string()))?;
        Ok(Self {
            sender,
            health,
            driver: Mutex::new(Some(driver)),
        })
    }

    /// Establishes a TCP stream through the userspace network stack.
    pub fn dial_context(
        &self,
        context: &DialContext,
        network: &str,
        address: &str,
    ) -> Result<TcpStream, TunnelError> {
        self.health.check()?;
        let remote = parse_tcp_endpoint(network, address)?;
        let (reply_tx, reply_rx) = mpsc::channel();
        self.send(Command::Open {
            remote,
            reply: reply_tx,
        })?;
        let opened = receive_driver(&reply_rx, &self.health)??;
        let stream = TcpStream {
            id: opened.id,
            sender: self.sender.clone(),
            health: self.health.clone(),
            signal: opened.signal,
            nonblocking: false,
            read_timeout: None,
            write_timeout: None,
        };

        loop {
            match stream.status()? {
                StreamStatus::Connected => return Ok(stream),
                StreamStatus::Connecting => {
                    let remaining = context.remaining()?;
                    let wait = remaining.map_or(DRIVER_TICK, |duration| duration.min(DRIVER_TICK));
                    stream.signal.wait(wait);
                }
                StreamStatus::Closed => {
                    return Err(TunnelError::dial("connection closed before establishment"));
                }
            }
            if let Err(error) = context.remaining() {
                stream.abort();
                return Err(error);
            }
        }
    }

    /// Stops the driver and closes every stream. Calling this repeatedly is safe.
    pub fn close(&self) {
        let driver = self
            .driver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(driver) = driver {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = self.sender.send(Command::Stop { reply: reply_tx });
            let _ = reply_rx.recv_timeout(Duration::from_secs(1));
            let _ = driver.join();
            self.health.close();
        }
    }

    fn send(&self, command: Command) -> Result<(), TunnelError> {
        self.sender
            .send(command)
            .map_err(|_| self.health.error_or_closed())
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.close();
    }
}

/// A full-duplex TCP stream carried inside the tunnel.
pub struct TcpStream {
    id: u64,
    sender: SyncSender<Command>,
    health: Arc<Health>,
    signal: Arc<Signal>,
    nonblocking: bool,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

impl fmt::Debug for TcpStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TcpStream")
            .field("id", &self.id)
            .field("nonblocking", &self.nonblocking)
            .field("read_timeout", &self.read_timeout)
            .field("write_timeout", &self.write_timeout)
            .finish_non_exhaustive()
    }
}

impl TcpStream {
    /// Enables or disables nonblocking standard I/O operations.
    pub fn set_nonblocking(&mut self, nonblocking: bool) {
        self.nonblocking = nonblocking;
    }

    /// Sets the maximum time a blocking read waits for progress.
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) {
        self.read_timeout = timeout;
    }

    /// Sets the maximum time a blocking write waits for progress.
    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) {
        self.write_timeout = timeout;
    }

    /// Closes one or both halves of the connection.
    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.command(Command::Shutdown {
            id: self.id,
            how,
            reply: reply_tx,
        })?;
        receive_driver_io(&reply_rx, &self.health)?
    }

    /// Polls a read without requiring a particular async runtime.
    pub fn poll_read(
        &self,
        context: &mut TaskContext<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match self.try_read(buffer) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.signal.register_read(context.waker());
                match self.try_read(buffer) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
                    result => Poll::Ready(result),
                }
            }
            result => Poll::Ready(result),
        }
    }

    /// Polls a write without requiring a particular async runtime.
    pub fn poll_write(
        &self,
        context: &mut TaskContext<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.try_write(buffer) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.signal.register_write(context.waker());
                match self.try_write(buffer) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
                    result => Poll::Ready(result),
                }
            }
            result => Poll::Ready(result),
        }
    }

    fn status(&self) -> Result<StreamStatus, TunnelError> {
        self.health.check()?;
        Ok(self.signal.status())
    }

    fn try_read(&self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.health.check().map_err(to_io_error)?;
        self.signal.try_read(buffer)
    }

    fn try_write(&self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.health.check().map_err(to_io_error)?;
        self.signal.try_write(buffer)
    }

    fn command(&self, command: Command) -> io::Result<()> {
        self.health.check().map_err(to_io_error)?;
        self.sender
            .send(command)
            .map_err(|_| self.health.io_error())
    }

    fn wait_for_progress(&self, deadline: Option<Instant>) -> io::Result<()> {
        if self.nonblocking {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let wait = deadline.map_or(DRIVER_TICK, |deadline| {
            deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_default()
                .min(DRIVER_TICK)
        });
        if wait.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.signal.wait(wait);
        self.health.check().map_err(to_io_error)
    }

    fn abort(&self) {
        self.signal.request_abort();
    }
}

impl Read for TcpStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let deadline = self
            .read_timeout
            .and_then(|timeout| Instant::now().checked_add(timeout));
        loop {
            match self.try_read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.wait_for_progress(deadline)?;
                }
                result => return result,
            }
        }
    }
}

impl Write for TcpStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let deadline = self
            .write_timeout
            .and_then(|timeout| Instant::now().checked_add(timeout));
        loop {
            match self.try_write(buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.wait_for_progress(deadline)?;
                }
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.health.check().map_err(to_io_error)
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        self.signal.request_drop();
    }
}

struct ValidatedConfig {
    local_address: IpAddr,
    private_key: StaticSecret,
    endpoint: SocketAddr,
    public_key: PublicKey,
    remote_network: IpPrefix,
    #[allow(dead_code)]
    dns: IpAddr,
    mtu: usize,
    keep_alive_seconds: u16,
}

impl TryFrom<TunnelConfig> for ValidatedConfig {
    type Error = TunnelError;

    fn try_from(config: TunnelConfig) -> Result<Self, Self::Error> {
        if config.local_address.is_unspecified() || config.local_address.is_multicast() {
            return Err(TunnelError::create(format!(
                "invalid local address {}",
                config.local_address
            )));
        }
        if config.endpoint.port() == 0 {
            return Err(TunnelError::configure("endpoint port must not be zero"));
        }
        if config.remote_network.address().is_unspecified()
            || config.remote_network.address().is_multicast()
        {
            return Err(TunnelError::configure(format!(
                "invalid allowed IP {}",
                config.remote_network.address()
            )));
        }
        if config.local_address.is_ipv4() != config.remote_network.address().is_ipv4() {
            return Err(TunnelError::create(
                "local address and remote network use different IP families",
            ));
        }
        if !(576..=u16::MAX as usize).contains(&config.mtu) {
            return Err(TunnelError::create(format!(
                "MTU {} is outside 576..=65535",
                config.mtu
            )));
        }
        let keep_alive_seconds = u16::try_from(config.keep_alive.as_secs()).map_err(|_| {
            TunnelError::configure("keepalive interval exceeds 65535 whole seconds")
        })?;
        let private_key = take_private_key(config.local_private_key)?;
        let public_key = PublicKey::from(exact_key(&config.remote_public_key, "public")?);
        let dns = config.dns.unwrap_or(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));

        Ok(Self {
            local_address: config.local_address,
            private_key,
            endpoint: config.endpoint,
            public_key,
            remote_network: config.remote_network,
            dns,
            mtu: config.mtu,
            keep_alive_seconds,
        })
    }
}

fn apply_defaults(config: &mut TunnelConfig) {
    if config.mtu == 0 {
        config.mtu = DEFAULT_MTU;
    }
    if config.keep_alive.is_zero() {
        config.keep_alive = DEFAULT_KEEPALIVE_INTERVAL;
    }
}

fn exact_key(secret: &Secret, description: &str) -> Result<[u8; 32], TunnelError> {
    secret.as_bytes().try_into().map_err(|_| {
        TunnelError::configure(format!(
            "{description} key must contain exactly 32 bytes, got {}",
            secret.as_bytes().len()
        ))
    })
}

fn take_private_key(mut secret: Secret) -> Result<StaticSecret, TunnelError> {
    let mut bytes = exact_key(&secret, "private")?;
    secret.as_mut_bytes().fill(0);
    let private_key = StaticSecret::from(bytes);
    bytes.fill(0);
    Ok(private_key)
}

fn create_udp_socket(endpoint: SocketAddr) -> Result<UdpSocket, TunnelError> {
    let bind_address = match endpoint {
        SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        SocketAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let socket = UdpSocket::bind(bind_address)
        .map_err(|error| TunnelError::create(format!("bind UDP socket: {error}")))?;
    socket
        .connect(endpoint)
        .map_err(|error| TunnelError::configure(format!("endpoint {endpoint}: {error}")))?;
    socket
        .set_nonblocking(true)
        .map_err(|error| TunnelError::enable(format!("set UDP socket nonblocking: {error}")))?;
    Ok(socket)
}

fn parse_tcp_endpoint(network: &str, address: &str) -> Result<SocketAddr, TunnelError> {
    if !matches!(network, "tcp" | "tcp4" | "tcp6") {
        return Err(TunnelError::dial(format!(
            "unsupported network {network:?}; only TCP is available"
        )));
    }
    let endpoint = SocketAddr::from_str(address).map_err(|_| {
        TunnelError::dial(format!(
            "address {address:?} is not a numeric IP socket address; in-tunnel DNS is unavailable"
        ))
    })?;
    if (network == "tcp4" && !endpoint.is_ipv4()) || (network == "tcp6" && !endpoint.is_ipv6()) {
        return Err(TunnelError::dial(format!(
            "address {address:?} does not match network {network:?}"
        )));
    }
    Ok(endpoint)
}

#[derive(Default)]
struct Health {
    state: Mutex<HealthState>,
}

#[derive(Default)]
struct HealthState {
    closed: bool,
    failure: Option<String>,
}

impl Health {
    fn check(&self) -> Result<(), TunnelError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(failure) = &state.failure {
            Err(TunnelError::dial(failure.clone()))
        } else if state.closed {
            Err(TunnelError::dial("tunnel is closed"))
        } else {
            Ok(())
        }
    }

    fn fail(&self, detail: impl Into<String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.failure.get_or_insert_with(|| detail.into());
    }

    fn close(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .closed = true;
    }

    fn error_or_closed(&self) -> TunnelError {
        self.check()
            .err()
            .unwrap_or_else(|| TunnelError::dial("tunnel driver stopped"))
    }

    fn io_error(&self) -> io::Error {
        to_io_error(self.error_or_closed())
    }
}

fn to_io_error(error: TunnelError) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, error)
}

fn receive_driver<T>(receiver: &Receiver<T>, health: &Health) -> Result<T, TunnelError> {
    loop {
        match receiver.recv_timeout(DRIVER_TICK) {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Timeout) => health.check()?,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(health.error_or_closed());
            }
        }
    }
}

fn receive_driver_io<T>(receiver: &Receiver<T>, health: &Health) -> io::Result<T> {
    receive_driver(receiver, health).map_err(to_io_error)
}

#[derive(Default)]
struct Signal {
    state: Mutex<SignalState>,
    changed: Condvar,
}

#[derive(Default)]
struct SignalState {
    generation: u64,
    incoming: VecDeque<u8>,
    outgoing: VecDeque<u8>,
    connected: bool,
    read_closed: bool,
    write_closed: bool,
    write_shutdown: bool,
    abort_requested: bool,
    dropped: bool,
    read_waker: Option<Waker>,
    write_waker: Option<Waker>,
}

impl Signal {
    fn notify_all(&self) {
        let wakers = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.generation = state.generation.wrapping_add(1);
            (state.read_waker.take(), state.write_waker.take())
        };
        self.changed.notify_all();
        if let Some(waker) = wakers.0 {
            waker.wake();
        }
        if let Some(waker) = wakers.1 {
            waker.wake();
        }
    }

    fn notify_read(&self) {
        let waker = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.generation = state.generation.wrapping_add(1);
            state.read_waker.take()
        };
        self.changed.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn notify_write(&self) {
        let waker = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.generation = state.generation.wrapping_add(1);
            state.write_waker.take()
        };
        self.changed.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn wait(&self, timeout: Duration) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let generation = state.generation;
        let (state, result) = self
            .changed
            .wait_timeout_while(state, timeout, |state| state.generation == generation)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.generation != generation || !result.timed_out()
    }

    fn register_read(&self, waker: &Waker) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .read_waker = Some(waker.clone());
    }

    fn register_write(&self, waker: &Waker) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .write_waker = Some(waker.clone());
    }

    fn status(&self) -> StreamStatus {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.connected && !(state.read_closed && state.write_closed) {
            StreamStatus::Connected
        } else if state.read_closed || state.write_closed {
            StreamStatus::Closed
        } else {
            StreamStatus::Connecting
        }
    }

    fn try_read(&self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let count = buffer.len().min(state.incoming.len());
        for (destination, byte) in buffer.iter_mut().zip(state.incoming.drain(..count)) {
            *destination = byte;
        }
        if count != 0 {
            Ok(count)
        } else if state.read_closed {
            Ok(0)
        } else {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    fn try_write(&self, buffer: &[u8]) -> io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.write_shutdown || state.write_closed {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let count = buffer
            .len()
            .min(TCP_BUFFER_SIZE.saturating_sub(state.outgoing.len()));
        if count == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        state.outgoing.extend(&buffer[..count]);
        Ok(count)
    }

    fn request_abort(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.abort_requested = true;
        state.read_closed = true;
        state.write_closed = true;
        state.incoming.clear();
        state.outgoing.clear();
        drop(state);
        self.notify_all();
    }

    fn request_drop(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.dropped = true;
        state.write_shutdown = true;
        drop(state);
        self.notify_all();
    }
}

enum Command {
    Open {
        remote: SocketAddr,
        reply: Sender<Result<OpenedStream, TunnelError>>,
    },
    Shutdown {
        id: u64,
        how: Shutdown,
        reply: Sender<io::Result<()>>,
    },
    Stop {
        reply: Sender<()>,
    },
}

struct OpenedStream {
    id: u64,
    signal: Arc<Signal>,
}

#[derive(Clone, Copy)]
enum StreamStatus {
    Connecting,
    Connected,
    Closed,
}

struct StreamRecord {
    handle: SocketHandle,
    signal: Arc<Signal>,
    connected: bool,
    local_port: u16,
    lifecycle: StreamLifecycle,
}

enum StreamLifecycle {
    Attached,
    Closing { deadline: Instant },
    Aborting { polls_remaining: u8 },
}

struct Driver {
    config: ValidatedConfig,
    udp: UdpSocket,
    commands: Receiver<Command>,
    health: Arc<Health>,
    tunnel: Tunn,
    device: IpDevice,
    interface: Interface,
    sockets: SocketSet<'static>,
    streams: BTreeMap<u64, StreamRecord>,
    used_ports: BTreeSet<u16>,
    next_stream_id: u64,
    next_port: u16,
    pending_datagrams: VecDeque<Vec<u8>>,
    needs_decapsulate_drain: bool,
    last_timer: Instant,
    stop_reply: Option<Sender<()>>,
}

impl Driver {
    fn new(
        config: ValidatedConfig,
        udp: UdpSocket,
        commands: Receiver<Command>,
        health: Arc<Health>,
    ) -> Self {
        let mut device = IpDevice::new(config.mtu);
        let mut interface_config = InterfaceConfig::new(HardwareAddress::Ip);
        interface_config.random_seed = random_seed();
        let mut interface = Interface::new(interface_config, &mut device, SmolInstant::now());
        let local_prefix = if config.local_address.is_ipv4() {
            32
        } else {
            128
        };
        interface.update_ip_addrs(|addresses| {
            addresses
                .push(IpCidr::new(config.local_address.into(), local_prefix))
                .expect("the configured address capacity always has one free slot");
        });
        interface.routes_mut().update(|routes| {
            routes
                .push(Route {
                    cidr: config.remote_network.as_cidr(),
                    via_router: config.remote_network.address().into(),
                    preferred_until: None,
                    expires_at: None,
                })
                .expect("the route capacity always has one free slot");
        });

        let tunnel = Tunn::new(
            config.private_key.clone(),
            config.public_key,
            None,
            Some(config.keep_alive_seconds),
            1,
            None,
        );
        Self {
            config,
            udp,
            commands,
            health,
            tunnel,
            device,
            interface,
            sockets: SocketSet::new(Vec::new()),
            streams: BTreeMap::new(),
            used_ports: BTreeSet::new(),
            next_stream_id: 1,
            next_port: 49_152,
            pending_datagrams: VecDeque::new(),
            needs_decapsulate_drain: false,
            last_timer: Instant::now(),
            stop_reply: None,
        }
    }

    fn run(mut self) {
        loop {
            if self.process_commands() {
                break;
            }
            if let Err(error) = self.flush_datagrams() {
                self.health.fail(error);
                break;
            }
            if let Err(error) = self.receive_datagrams() {
                self.health.fail(error);
                break;
            }
            self.sync_streams();
            self.poll_stack();
            self.sync_streams();
            if let Err(error) = self.send_plaintext() {
                self.health.fail(error);
                break;
            }
            if self.last_timer.elapsed() >= TIMER_TICK {
                if let Err(error) = self.update_timers() {
                    self.health.fail(error);
                    break;
                }
                self.last_timer = Instant::now();
            }
            if let Err(error) = self.flush_datagrams() {
                self.health.fail(error);
                break;
            }
            self.cleanup_streams();
            thread::sleep(DRIVER_TICK);
        }
        for stream in self.streams.values() {
            stream.signal.notify_all();
        }
        self.health.close();
        if let Some(reply) = self.stop_reply.take() {
            let _ = reply.send(());
        }
    }

    fn process_commands(&mut self) -> bool {
        for _ in 0..MAX_COMMANDS {
            let Ok(command) = self.commands.try_recv() else {
                break;
            };
            match command {
                Command::Open { remote, reply } => {
                    let result = self.open(remote);
                    let _ = reply.send(result);
                }
                Command::Shutdown { id, how, reply } => {
                    let result = self.shutdown(id, how);
                    let _ = reply.send(result);
                }
                Command::Stop { reply } => {
                    self.stop_reply = Some(reply);
                    return true;
                }
            }
        }
        false
    }

    fn open(&mut self, remote: SocketAddr) -> Result<OpenedStream, TunnelError> {
        if self.streams.len() >= MAX_STREAMS {
            return Err(TunnelError::dial(format!(
                "at most {MAX_STREAMS} simultaneous streams are supported"
            )));
        }
        if !self.config.remote_network.contains(remote.ip()) {
            return Err(TunnelError::dial(format!(
                "address {} is outside allowed network {}/{}",
                remote.ip(),
                self.config.remote_network.address(),
                self.config.remote_network.prefix_len()
            )));
        }
        let local_port = self
            .allocate_port()
            .ok_or_else(|| TunnelError::dial("no ephemeral TCP ports are available"))?;
        let rx = tcp::SocketBuffer::new(vec![0; TCP_BUFFER_SIZE]);
        let tx = tcp::SocketBuffer::new(vec![0; TCP_BUFFER_SIZE]);
        let handle = self.sockets.add(tcp::Socket::new(rx, tx));
        if let Err(error) = self.sockets.get_mut::<tcp::Socket>(handle).connect(
            self.interface.context(),
            IpEndpoint::new(remote.ip().into(), remote.port()),
            local_port,
        ) {
            self.sockets.remove(handle);
            return Err(TunnelError::dial(format!("connect {remote}: {error}")));
        }
        self.used_ports.insert(local_port);
        let id = self.next_stream_id;
        self.next_stream_id = self.next_stream_id.wrapping_add(1).max(1);
        let signal = Arc::new(Signal::default());
        self.streams.insert(
            id,
            StreamRecord {
                handle,
                signal: signal.clone(),
                connected: false,
                local_port,
                lifecycle: StreamLifecycle::Attached,
            },
        );
        Ok(OpenedStream { id, signal })
    }

    fn allocate_port(&mut self) -> Option<u16> {
        for _ in 49_152..=u16::MAX {
            let port = self.next_port;
            self.next_port = if port == u16::MAX { 49_152 } else { port + 1 };
            if !self.used_ports.contains(&port) {
                return Some(port);
            }
        }
        None
    }

    fn shutdown(&mut self, id: u64, how: Shutdown) -> io::Result<()> {
        let record = self.streams.get(&id).ok_or_else(closed_stream)?;
        match how {
            Shutdown::Read => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "smoltcp cannot shut down only the receive half",
                ));
            }
            Shutdown::Write => {
                record
                    .signal
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .write_shutdown = true;
            }
            Shutdown::Both => record.signal.request_abort(),
        }
        record.signal.notify_all();
        Ok(())
    }

    fn sync_streams(&mut self) {
        let mut notifications = Vec::new();
        for record in self.streams.values_mut() {
            let socket = self.sockets.get_mut::<tcp::Socket>(record.handle);
            let mut state = record
                .signal
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let was_read_ready = !state.incoming.is_empty() || state.read_closed;
            let was_write_ready = state.outgoing.len() < TCP_BUFFER_SIZE || state.write_closed;

            if state.abort_requested {
                socket.abort();
                state.read_closed = true;
                state.write_closed = true;
                if state.dropped && matches!(record.lifecycle, StreamLifecycle::Attached) {
                    record.lifecycle = StreamLifecycle::Aborting { polls_remaining: 1 };
                }
            } else {
                if socket.can_send() && !state.outgoing.is_empty() {
                    let bytes = state.outgoing.make_contiguous();
                    if let Ok(count) = socket.send_slice(bytes) {
                        state.outgoing.drain(..count);
                    }
                }

                if state.write_shutdown && state.outgoing.is_empty() && socket.may_send() {
                    socket.close();
                    state.write_closed = true;
                    if state.dropped && matches!(record.lifecycle, StreamLifecycle::Attached) {
                        record.lifecycle = StreamLifecycle::Closing {
                            deadline: Instant::now() + CLOSE_TIMEOUT,
                        };
                    }
                } else if state.dropped
                    && !record.connected
                    && matches!(record.lifecycle, StreamLifecycle::Attached)
                {
                    socket.abort();
                    state.read_closed = true;
                    state.write_closed = true;
                    record.lifecycle = StreamLifecycle::Aborting { polls_remaining: 1 };
                }

                if state.dropped
                    && record.connected
                    && !socket.may_send()
                    && matches!(record.lifecycle, StreamLifecycle::Attached)
                {
                    record.lifecycle = StreamLifecycle::Closing {
                        deadline: Instant::now() + CLOSE_TIMEOUT,
                    };
                }

                let available = TCP_BUFFER_SIZE.saturating_sub(state.incoming.len());
                if available != 0 && socket.can_recv() {
                    let mut bytes = vec![0; available.min(8 * 1024)];
                    if let Ok(count) = socket.recv_slice(&mut bytes) {
                        state.incoming.extend(&bytes[..count]);
                    }
                }

                record.connected |= socket.may_send();
                state.connected = record.connected;
                if record.connected && !socket.may_recv() && state.incoming.is_empty() {
                    state.read_closed = true;
                }
                if record.connected && !socket.may_send() {
                    state.write_closed = true;
                }
                if !record.connected && !socket.is_open() {
                    state.read_closed = true;
                    state.write_closed = true;
                }
            }

            let read_ready = !state.incoming.is_empty() || state.read_closed;
            let write_ready = state.outgoing.len() < TCP_BUFFER_SIZE || state.write_closed;
            notifications.push((
                record.signal.clone(),
                !was_read_ready && read_ready,
                !was_write_ready && write_ready,
            ));
        }

        for (signal, read, write) in notifications {
            match (read, write) {
                (true, true) => signal.notify_all(),
                (true, false) => signal.notify_read(),
                (false, true) => signal.notify_write(),
                (false, false) => {}
            }
        }
    }

    fn cleanup_streams(&mut self) {
        let now = Instant::now();
        let mut remove = Vec::new();
        for (&id, record) in &mut self.streams {
            let socket = self.sockets.get_mut::<tcp::Socket>(record.handle);
            match &mut record.lifecycle {
                StreamLifecycle::Attached => {}
                StreamLifecycle::Closing { .. } if socket.state() == tcp::State::Closed => {
                    remove.push(id);
                }
                StreamLifecycle::Closing { deadline } if now >= *deadline => {
                    socket.abort();
                    record.lifecycle = StreamLifecycle::Aborting { polls_remaining: 1 };
                }
                StreamLifecycle::Closing { .. } => {}
                StreamLifecycle::Aborting { polls_remaining } if *polls_remaining == 0 => {
                    remove.push(id);
                }
                StreamLifecycle::Aborting { polls_remaining } => {
                    *polls_remaining -= 1;
                }
            }
        }
        for id in remove {
            if let Some(record) = self.streams.remove(&id) {
                self.sockets.remove(record.handle);
                self.used_ports.remove(&record.local_port);
                record.signal.notify_all();
            }
        }
    }

    fn receive_datagrams(&mut self) -> Result<(), String> {
        if self.pending_datagrams.len() >= MAX_PENDING_DATAGRAMS {
            return Ok(());
        }
        if self.needs_decapsulate_drain {
            self.needs_decapsulate_drain = false;
            self.decapsulate(&[])?;
            if self.pending_datagrams.len() >= MAX_PENDING_DATAGRAMS {
                return Ok(());
            }
        }
        let mut datagram = vec![0; MAX_UDP_DATAGRAM];
        for _ in 0..MAX_DATAGRAMS_PER_TICK {
            match self.udp.recv(&mut datagram) {
                Ok(0) => {}
                Ok(length) => self.decapsulate(&datagram[..length])?,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(format!("receive from WireGuard endpoint: {error}")),
            }
            if self.pending_datagrams.len() >= MAX_PENDING_DATAGRAMS {
                break;
            }
        }
        Ok(())
    }

    fn decapsulate(&mut self, datagram: &[u8]) -> Result<(), String> {
        let mut output = vec![0; MAX_UDP_DATAGRAM + 32];
        let mut input = datagram;
        loop {
            if self.pending_datagrams.len() >= MAX_PENDING_DATAGRAMS {
                self.needs_decapsulate_drain = true;
                return Ok(());
            }
            match self
                .tunnel
                .decapsulate(Some(self.config.endpoint.ip()), input, &mut output)
            {
                TunnResult::Done => return Ok(()),
                TunnResult::Err(WireGuardError::ConnectionExpired) => {
                    self.reset_tunnel();
                    return Ok(());
                }
                TunnResult::Err(_) => return Ok(()),
                TunnResult::WriteToNetwork(packet) => {
                    let packet = packet.to_vec();
                    self.queue_datagram(packet)?;
                }
                TunnResult::WriteToTunnelV4(packet, source) => {
                    self.accept_plaintext(packet, IpAddr::V4(source));
                }
                TunnResult::WriteToTunnelV6(packet, source) => {
                    self.accept_plaintext(packet, IpAddr::V6(source));
                }
            }
            input = &[];
        }
    }

    fn accept_plaintext(&mut self, packet: &[u8], source: IpAddr) {
        if self.config.remote_network.contains(source)
            && packet_destination(packet) == Some(self.config.local_address)
            && packet.len() <= self.config.mtu
            && self.device.incoming.len() < MAX_PLAINTEXT_PACKETS
        {
            self.device.incoming.push_back(packet.to_vec());
        }
    }

    fn poll_stack(&mut self) {
        let _ = self
            .interface
            .poll(SmolInstant::now(), &mut self.device, &mut self.sockets);
    }

    fn send_plaintext(&mut self) -> Result<(), String> {
        let mut output = vec![0; self.config.mtu + 256];
        while self.pending_datagrams.len() < MAX_PENDING_DATAGRAMS {
            let Some(packet) = self.device.outgoing.pop_front() else {
                break;
            };
            let destination = packet_destination(&packet);
            if !destination.is_some_and(|address| self.config.remote_network.contains(address)) {
                continue;
            }
            match self.tunnel.encapsulate(&packet, &mut output) {
                TunnResult::WriteToNetwork(encrypted) => {
                    let encrypted = encrypted.to_vec();
                    self.queue_datagram(encrypted)?;
                }
                TunnResult::Done => {}
                TunnResult::Err(WireGuardError::ConnectionExpired) => {
                    self.reset_tunnel();
                    self.device.outgoing.push_front(packet);
                    break;
                }
                TunnResult::Err(error) => {
                    return Err(format!("encapsulate WireGuard packet: {error:?}"));
                }
                TunnResult::WriteToTunnelV4(_, _) | TunnResult::WriteToTunnelV6(_, _) => {
                    return Err("WireGuard encapsulation returned plaintext".into());
                }
            }
        }
        Ok(())
    }

    fn update_timers(&mut self) -> Result<(), String> {
        if self.pending_datagrams.len() >= MAX_PENDING_DATAGRAMS {
            return Ok(());
        }
        let mut output = vec![0; self.config.mtu + 256];
        match self.tunnel.update_timers(&mut output) {
            TunnResult::Done => Ok(()),
            TunnResult::WriteToNetwork(packet) => {
                let packet = packet.to_vec();
                self.queue_datagram(packet)
            }
            TunnResult::Err(WireGuardError::ConnectionExpired) => {
                self.reset_tunnel();
                Ok(())
            }
            TunnResult::Err(error) => Err(format!("WireGuard timer: {error:?}")),
            TunnResult::WriteToTunnelV4(_, _) | TunnResult::WriteToTunnelV6(_, _) => {
                Err("WireGuard timer returned plaintext".into())
            }
        }
    }

    fn queue_datagram(&mut self, datagram: Vec<u8>) -> Result<(), String> {
        if self.pending_datagrams.is_empty() {
            match self.udp.send(&datagram) {
                Ok(length) if length == datagram.len() => return Ok(()),
                Ok(length) => {
                    return Err(format!(
                        "send WireGuard datagram: wrote {length} of {} bytes",
                        datagram.len()
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(format!("send WireGuard datagram: {error}")),
            }
        }
        if self.pending_datagrams.len() >= MAX_PENDING_DATAGRAMS {
            return Err("WireGuard datagram queue capacity was exceeded".into());
        }
        self.pending_datagrams.push_back(datagram);
        Ok(())
    }

    fn flush_datagrams(&mut self) -> Result<(), String> {
        while let Some(datagram) = self.pending_datagrams.front() {
            match self.udp.send(datagram) {
                Ok(length) if length == datagram.len() => {
                    self.pending_datagrams.pop_front();
                }
                Ok(length) => {
                    return Err(format!(
                        "send WireGuard datagram: wrote {length} of {} bytes",
                        datagram.len()
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(format!("send WireGuard datagram: {error}")),
            }
        }
        Ok(())
    }

    fn reset_tunnel(&mut self) {
        self.tunnel = Tunn::new(
            self.config.private_key.clone(),
            self.config.public_key,
            None,
            Some(self.config.keep_alive_seconds),
            1,
            None,
        );
        self.pending_datagrams.clear();
        self.needs_decapsulate_drain = false;
    }
}

fn random_seed() -> u64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    duration.as_secs() ^ u64::from(duration.subsec_nanos()).rotate_left(32)
}

fn closed_stream() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "tunnel TCP stream is closed",
    )
}

fn packet_destination(packet: &[u8]) -> Option<IpAddr> {
    match packet.first().map(|byte| byte >> 4) {
        Some(4) if packet.len() >= 20 => Some(IpAddr::V4(Ipv4Addr::new(
            packet[16], packet[17], packet[18], packet[19],
        ))),
        Some(6) if packet.len() >= 40 => {
            let octets: [u8; 16] = packet[24..40].try_into().ok()?;
            Some(IpAddr::V6(Ipv6Addr::from(octets)))
        }
        _ => None,
    }
}

struct IpDevice {
    incoming: VecDeque<Vec<u8>>,
    outgoing: VecDeque<Vec<u8>>,
    mtu: usize,
}

impl IpDevice {
    fn new(mtu: usize) -> Self {
        Self {
            incoming: VecDeque::new(),
            outgoing: VecDeque::new(),
            mtu,
        }
    }
}

impl Device for IpDevice {
    type RxToken<'a> = IpRxToken;
    type TxToken<'a> = IpTxToken<'a>;

    fn receive(
        &mut self,
        _timestamp: SmolInstant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if self.outgoing.len() >= MAX_PLAINTEXT_PACKETS {
            return None;
        }
        self.incoming.pop_front().map(|buffer| {
            (
                IpRxToken(buffer),
                IpTxToken {
                    queue: &mut self.outgoing,
                },
            )
        })
    }

    fn transmit(&mut self, _timestamp: SmolInstant) -> Option<Self::TxToken<'_>> {
        (self.outgoing.len() < MAX_PLAINTEXT_PACKETS).then_some(IpTxToken {
            queue: &mut self.outgoing,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.max_transmission_unit = self.mtu;
        capabilities.medium = Medium::Ip;
        capabilities
    }
}

struct IpRxToken(Vec<u8>);

impl phy::RxToken for IpRxToken {
    fn consume<R, F>(self, function: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        function(&self.0)
    }
}

struct IpTxToken<'a> {
    queue: &'a mut VecDeque<Vec<u8>>,
}

impl phy::TxToken for IpTxToken<'_> {
    fn consume<R, F>(self, length: usize, function: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = vec![0; length];
        let result = function(&mut buffer);
        self.queue.push_back(buffer);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn config(endpoint: SocketAddr) -> TunnelConfig {
        TunnelConfig {
            local_address: "fd00::1".parse().unwrap(),
            local_private_key: Secret::from([7; 32]),
            endpoint,
            remote_public_key: Secret::from([9; 32]),
            remote_network: IpPrefix::new("fd00::2".parse().unwrap(), 128).unwrap(),
            dns: None,
            mtu: 0,
            keep_alive: Duration::ZERO,
        }
    }

    fn public_key(private_key: [u8; 32]) -> [u8; 32] {
        PublicKey::from(&StaticSecret::from(private_key)).to_bytes()
    }

    fn run_echo_peer(
        udp: UdpSocket,
        stop: Arc<AtomicBool>,
        closed_streams: Arc<AtomicUsize>,
        private_key: [u8; 32],
        client_public_key: [u8; 32],
    ) {
        udp.set_nonblocking(true).unwrap();
        let peer_address: IpAddr = "fd00::2".parse().unwrap();
        let client_address: IpAddr = "fd00::1".parse().unwrap();
        let mut wireguard = Tunn::new(
            StaticSecret::from(private_key),
            PublicKey::from(client_public_key),
            None,
            Some(25),
            2,
            None,
        );
        let mut device = IpDevice::new(DEFAULT_MTU);
        let mut interface = Interface::new(
            InterfaceConfig::new(HardwareAddress::Ip),
            &mut device,
            SmolInstant::now(),
        );
        interface.update_ip_addrs(|addresses| {
            addresses
                .push(IpCidr::new(peer_address.into(), 128))
                .unwrap();
        });
        interface.routes_mut().update(|routes| {
            routes
                .push(Route {
                    cidr: IpCidr::new(client_address.into(), 128),
                    via_router: client_address.into(),
                    preferred_until: None,
                    expires_at: None,
                })
                .unwrap();
        });
        let mut sockets = SocketSet::new(Vec::new());
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let handle = sockets.add(tcp::Socket::new(
                    tcp::SocketBuffer::new(vec![0; TCP_BUFFER_SIZE]),
                    tcp::SocketBuffer::new(vec![0; TCP_BUFFER_SIZE]),
                ));
                sockets
                    .get_mut::<tcp::Socket>(handle)
                    .listen((peer_address, 8080))
                    .unwrap();
                handle
            })
            .collect();

        let mut client_endpoint = None;
        let mut datagram = vec![0; MAX_UDP_DATAGRAM];
        let mut output = vec![0; MAX_UDP_DATAGRAM + 32];
        let mut pending_replies = vec![false; handles.len()];
        let mut close_seen = vec![false; handles.len()];
        let mut last_timer = Instant::now();
        while !stop.load(Ordering::Acquire) {
            loop {
                let (length, source) = match udp.recv_from(&mut datagram) {
                    Ok(received) => received,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("peer UDP receive failed: {error}"),
                };
                client_endpoint = Some(source);
                if length == 0 {
                    continue;
                }
                let mut input = &datagram[..length];
                loop {
                    match wireguard.decapsulate(Some(source.ip()), input, &mut output) {
                        TunnResult::Done | TunnResult::Err(_) => break,
                        TunnResult::WriteToNetwork(packet) => {
                            udp.send_to(packet, source).unwrap();
                        }
                        TunnResult::WriteToTunnelV4(packet, _) => {
                            device.incoming.push_back(packet.to_vec());
                        }
                        TunnResult::WriteToTunnelV6(packet, _) => {
                            device.incoming.push_back(packet.to_vec());
                        }
                    }
                    input = &[];
                }
            }

            let _ = interface.poll(SmolInstant::now(), &mut device, &mut sockets);
            for (index, handle) in handles.iter().copied().enumerate() {
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                if socket.can_recv() {
                    let mut request = [0; 4];
                    if socket.recv_slice(&mut request).unwrap() == request.len() {
                        assert_eq!(&request, b"ping");
                        pending_replies[index] = true;
                    }
                }
                if pending_replies[index] && socket.can_send() {
                    assert_eq!(socket.send_slice(b"pong").unwrap(), 4);
                    pending_replies[index] = false;
                }
                if socket.state() == tcp::State::CloseWait && !close_seen[index] {
                    close_seen[index] = true;
                    closed_streams.fetch_add(1, Ordering::AcqRel);
                }
            }
            let _ = interface.poll(SmolInstant::now(), &mut device, &mut sockets);

            if let Some(destination) = client_endpoint {
                while let Some(packet) = device.outgoing.pop_front() {
                    match wireguard.encapsulate(&packet, &mut output) {
                        TunnResult::WriteToNetwork(encrypted) => {
                            udp.send_to(encrypted, destination).unwrap();
                        }
                        TunnResult::Done => {}
                        other => panic!("unexpected peer encapsulation result: {other:?}"),
                    }
                }
                if last_timer.elapsed() >= TIMER_TICK {
                    match wireguard.update_timers(&mut output) {
                        TunnResult::WriteToNetwork(packet) => {
                            udp.send_to(packet, destination).unwrap();
                        }
                        TunnResult::Done => {}
                        TunnResult::Err(WireGuardError::ConnectionExpired) => break,
                        other => panic!("unexpected peer timer result: {other:?}"),
                    }
                    last_timer = Instant::now();
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn oracle_defaults_are_applied() {
        let endpoint = "127.0.0.1:51820".parse().unwrap();
        let mut config = config(endpoint);
        apply_defaults(&mut config);
        assert_eq!(config.mtu, 1_420);
        assert_eq!(config.keep_alive, Duration::from_secs(25));
        let validated = ValidatedConfig::try_from(config).unwrap();
        assert_eq!(validated.dns, "1.1.1.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn invalid_keys_have_configuration_context() {
        let mut config = config("127.0.0.1:51820".parse().unwrap());
        config.local_private_key = Secret::from([0; 31]);
        apply_defaults(&mut config);
        let error = match ValidatedConfig::try_from(config) {
            Ok(_) => panic!("an invalid private key must be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "configure WireGuard device: private key must contain exactly 32 bytes, got 31"
        );
    }

    #[test]
    fn prefixes_cover_only_their_family_and_network_bits() {
        let all_v4 = IpPrefix::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0).unwrap();
        assert!(all_v4.contains("203.0.113.7".parse().unwrap()));
        assert!(!all_v4.contains("fd00::1".parse().unwrap()));

        let v4 = IpPrefix::new("10.42.0.7".parse().unwrap(), 16).unwrap();
        assert!(v4.contains("10.42.255.254".parse().unwrap()));
        assert!(!v4.contains("10.43.0.1".parse().unwrap()));
        assert!(!v4.contains("fd00::1".parse().unwrap()));

        let v6 = IpPrefix::new("fd42:1234::5".parse().unwrap(), 64).unwrap();
        assert!(v6.contains("fd42:1234::ffff".parse().unwrap()));
        assert!(!v6.contains("fd42:1235::1".parse().unwrap()));
    }

    #[test]
    fn numeric_tcp_scope_is_explicit() {
        assert_eq!(
            parse_tcp_endpoint("udp", "[fd00::2]:80")
                .unwrap_err()
                .to_string(),
            "dial through WireGuard tunnel: unsupported network \"udp\"; only TCP is available"
        );
        assert!(parse_tcp_endpoint("tcp", "machine.internal:80").is_err());
        assert!(parse_tcp_endpoint("tcp4", "[fd00::2]:80").is_err());
        assert_eq!(
            parse_tcp_endpoint("tcp6", "[fd00::2]:80").unwrap(),
            "[fd00::2]:80".parse().unwrap()
        );
    }

    #[test]
    fn stream_write_queue_is_bounded_and_polling_is_nonblocking() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let signal = Arc::new(Signal::default());
        {
            let mut state = signal.state.lock().unwrap();
            state.connected = true;
        }
        let stream = TcpStream {
            id: 1,
            sender,
            health: Arc::new(Health::default()),
            signal: signal.clone(),
            nonblocking: false,
            read_timeout: None,
            write_timeout: None,
        };
        let bytes = vec![0x5a; TCP_BUFFER_SIZE * 2];
        assert_eq!(stream.try_write(&bytes).unwrap(), TCP_BUFFER_SIZE);
        assert_eq!(
            stream.try_write(&[1]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );

        let waker = Waker::noop();
        let mut context = TaskContext::from_waker(waker);
        assert!(matches!(
            stream.poll_write(&mut context, &[1]),
            Poll::Pending
        ));
        let mut read = [0; 1];
        assert!(matches!(
            stream.poll_read(&mut context, &mut read),
            Poll::Pending
        ));
    }

    #[test]
    fn packet_destination_rejects_truncation() {
        assert_eq!(packet_destination(&[0x45; 19]), None);
        let mut ipv4 = [0_u8; 20];
        ipv4[0] = 0x45;
        ipv4[16..20].copy_from_slice(&[10, 0, 0, 2]);
        assert_eq!(packet_destination(&ipv4), Some("10.0.0.2".parse().unwrap()));

        assert_eq!(packet_destination(&[0x60; 39]), None);
        let mut ipv6 = [0_u8; 40];
        ipv6[0] = 0x60;
        ipv6[24..40].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());
        assert_eq!(
            packet_destination(&ipv6),
            Some(IpAddr::V6(Ipv6Addr::LOCALHOST))
        );
    }

    #[test]
    fn virtual_device_withholds_tokens_at_plaintext_capacity() {
        let mut device = IpDevice::new(DEFAULT_MTU);
        device.incoming.push_back(vec![0x45; 20]);
        device
            .outgoing
            .resize_with(MAX_PLAINTEXT_PACKETS, || vec![0x45; 20]);
        assert!(device.transmit(SmolInstant::now()).is_none());
        assert!(device.receive(SmolInstant::now()).is_none());
        device.outgoing.pop_front();
        assert!(device.transmit(SmolInstant::now()).is_some());
        assert!(device.receive(SmolInstant::now()).is_some());
    }

    #[test]
    fn close_is_idempotent_and_wakes_pending_dial() {
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = peer.local_addr().unwrap();
        let tunnel = Tunnel::connect(config(endpoint)).unwrap();
        tunnel.close();
        tunnel.close();
        let error = tunnel
            .dial_context(&DialContext::background(), "tcp", "[fd00::2]:80")
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "dial through WireGuard tunnel: tunnel is closed"
        );
    }

    #[test]
    fn cancelled_dial_returns_without_peer_response() {
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = peer.local_addr().unwrap();
        let tunnel = Tunnel::connect(config(endpoint)).unwrap();
        let context = DialContext::with_timeout(Duration::from_millis(40));
        let error = tunnel
            .dial_context(&context, "tcp", "[fd00::2]:80")
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "dial through WireGuard tunnel: deadline exceeded"
        );
        tunnel.close();
    }

    #[test]
    fn live_wireguard_handshake_carries_tcp_stream() {
        let peer_udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = peer_udp.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let closed_streams = Arc::new(AtomicUsize::new(0));
        let peer_private = [11; 32];
        let client_private = [7; 32];
        let peer_stop = stop.clone();
        let peer_closed_streams = closed_streams.clone();
        let peer = thread::spawn(move || {
            run_echo_peer(
                peer_udp,
                peer_stop,
                peer_closed_streams,
                peer_private,
                public_key(client_private),
            );
        });

        let mut tunnel_config = config(endpoint);
        tunnel_config.local_private_key = Secret::from(client_private);
        tunnel_config.remote_public_key = Secret::from(public_key(peer_private));
        let tunnel = Tunnel::connect(tunnel_config).unwrap();
        let mut first = tunnel
            .dial_context(
                &DialContext::with_timeout(Duration::from_secs(3)),
                "tcp",
                "[fd00::2]:8080",
            )
            .unwrap();
        let mut second = tunnel
            .dial_context(
                &DialContext::with_timeout(Duration::from_secs(3)),
                "tcp",
                "[fd00::2]:8080",
            )
            .unwrap();
        let read_shutdown = first.shutdown(Shutdown::Read).unwrap_err();
        assert_eq!(read_shutdown.kind(), io::ErrorKind::Unsupported);
        first.write_all(b"ping").unwrap();
        second.write_all(b"ping").unwrap();
        for stream in [&mut first, &mut second] {
            let mut response = [0; 4];
            stream.read_exact(&mut response).unwrap();
            assert_eq!(&response, b"pong");
        }
        first.shutdown(Shutdown::Write).unwrap();
        drop((first, second));
        let close_deadline = Instant::now() + Duration::from_secs(1);
        while closed_streams.load(Ordering::Acquire) != 2 && Instant::now() < close_deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(closed_streams.load(Ordering::Acquire), 2);
        tunnel.close();
        stop.store(true, Ordering::Release);
        peer.join().unwrap();
    }
}
