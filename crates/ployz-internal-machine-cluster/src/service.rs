use std::error::Error as StdError;
use std::fmt;
use std::net::IpAddr;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use ployz_internal_corrosion::{
    AdminClient, ClusterMembershipState, MemberRttStats, MembershipState,
};
use ployz_internal_dns::Client as DnsClient;
use ployz_internal_machine_api_pb::{
    AddMachineRequest, AddMachineResponse, Address, CreateDomainRecordsRequest,
    CreateDomainRecordsResponse, Domain, Ip, IpPrefix as PbIpPrefix, ListMachinesResponse,
    MachineInfo, MachineMember, NetworkConfig, RemoveMachineRequest, ReserveDomainRequest,
    cluster_server, machine_member,
};
use ployz_internal_machine_network::management_ip;
use ployz_internal_machine_store::{DeleteOptions, Error as StoreError, Store};
use ployz_internal_secret::Secret;
use tonic::{Code, Request, Response, Status};

use crate::dns::{
    DnsAccess, StoredDomain, encode_stored_domain, map_record_request, map_record_response,
};
use crate::{DEFAULT_SUBNET_BITS, IpPrefix, Ipam, new_machine_id, new_random_machine_name};

const NETWORK_KEY: &str = "network";
const CREATED_AT_KEY: &str = "created_at";
const UNCLOUD_DNS_KEY: &str = "uncloud_dns";

type BoxError = Box<dyn StdError + Send + Sync>;

/// A cloneable one-way signal matching a Go channel that is only ever closed.
#[derive(Clone, Debug, Default)]
pub struct Latch(Arc<AtomicBool>);

impl Latch {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn signal(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_signaled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
pub enum ClusterInitError {
    AlreadyInitialised,
    Store {
        context: &'static str,
        source: StoreError,
    },
}

impl fmt::Display for ClusterInitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyInitialised => {
                formatter.write_str("cluster is already initialised on this machine")
            }
            Self::Store { context, source } => write!(formatter, "{context}: {source}"),
        }
    }
}

impl StdError for ClusterInitError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Store { source, .. } => Some(source),
            Self::AlreadyInitialised => None,
        }
    }
}

#[tonic::async_trait]
trait StoreAccess: Send + Sync + 'static {
    async fn put_string(&self, key: &str, value: &str) -> Result<(), StoreError>;
    async fn put_bytes(&self, key: &str, value: &[u8]) -> Result<(), StoreError>;
    async fn get_string(&self, key: &str) -> Result<String, StoreError>;
    async fn get_bytes(&self, key: &str) -> Result<Vec<u8>, StoreError>;
    async fn delete(&self, key: &str) -> Result<(), StoreError>;
    async fn list_machines(&self) -> Result<Vec<MachineInfo>, StoreError>;
    async fn create_machine(&self, machine: &MachineInfo) -> Result<(), StoreError>;
    async fn delete_machine(&self, id: &str) -> Result<(), StoreError>;
    async fn delete_machine_containers(&self, id: &str) -> Result<(), StoreError>;
}

#[tonic::async_trait]
impl StoreAccess for Store {
    async fn put_string(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.put(key, value).await
    }

    async fn put_bytes(&self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        self.put(key, value).await
    }

    async fn get_string(&self, key: &str) -> Result<String, StoreError> {
        self.get(key).await
    }

    async fn get_bytes(&self, key: &str) -> Result<Vec<u8>, StoreError> {
        self.get(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        self.delete(key).await
    }

    async fn list_machines(&self) -> Result<Vec<MachineInfo>, StoreError> {
        self.list_machines().await
    }

    async fn create_machine(&self, machine: &MachineInfo) -> Result<(), StoreError> {
        self.create_machine(machine).await
    }

    async fn delete_machine(&self, id: &str) -> Result<(), StoreError> {
        self.delete_machine(id).await
    }

    async fn delete_machine_containers(&self, id: &str) -> Result<(), StoreError> {
        self.delete_containers(&DeleteOptions {
            machine_ids: vec![id.to_owned()],
            ..DeleteOptions::default()
        })
        .await
    }
}

#[tonic::async_trait]
trait AdminAccess: Send + Sync + 'static {
    async fn membership_states(&self) -> Result<Vec<ClusterMembershipState>, BoxError>;
    async fn member_rtts(&self) -> Result<Vec<MemberRttStats>, BoxError>;
}

#[tonic::async_trait]
impl AdminAccess for AdminClient {
    async fn membership_states(&self) -> Result<Vec<ClusterMembershipState>, BoxError> {
        self.cluster_membership_states(true)
            .await
            .map_err(|error| Box::new(error) as BoxError)
    }

    async fn member_rtts(&self) -> Result<Vec<MemberRttStats>, BoxError> {
        self.cluster_member_rtts()
            .await
            .map_err(|error| Box::new(error) as BoxError)
    }
}

/// Cluster gRPC service and the internal operations used during machine setup.
pub struct Cluster {
    store: Arc<dyn StoreAccess>,
    admin: Arc<dyn AdminAccess>,
    dns: Arc<dyn DnsAccess>,
    machine_id: RwLock<String>,
    initialised: Latch,
    ready: Latch,
}

impl Cluster {
    #[must_use]
    pub fn new(store: Store, admin: AdminClient, initialised: Latch, ready: Latch) -> Self {
        Self::with_dependencies(store, admin, DnsClient::new(), initialised, ready)
    }

    fn with_dependencies(
        store: impl StoreAccess,
        admin: impl AdminAccess,
        dns: impl DnsAccess,
        initialised: Latch,
        ready: Latch,
    ) -> Self {
        Self {
            store: Arc::new(store),
            admin: Arc::new(admin),
            dns: Arc::new(dns),
            machine_id: RwLock::new(String::new()),
            initialised,
            ready,
        }
    }

    pub fn update_machine_id(&self, machine_id: impl Into<String>) {
        *write_lock(&self.machine_id) = machine_id.into();
    }
    pub async fn init(&self, network: IpPrefix) -> Result<(), ClusterInitError> {
        if self.initialised.is_signaled() {
            return Err(ClusterInitError::AlreadyInitialised);
        }
        self.store
            .put_string(NETWORK_KEY, &network.to_string())
            .await
            .map_err(|source| ClusterInitError::Store {
                context: "put network to store",
                source,
            })?;
        let timestamp = rfc3339_utc(SystemTime::now());
        self.store
            .put_string(CREATED_AT_KEY, &timestamp)
            .await
            .map_err(|source| ClusterInitError::Store {
                context: "put created_at to store",
                source,
            })
    }

    pub async fn add_machine_without_ready_check(
        &self,
        request: AddMachineRequest,
    ) -> Result<AddMachineResponse, Status> {
        let network = request
            .network
            .ok_or_else(|| Status::invalid_argument("network not set"))?;
        network.validate()?;
        if network.endpoints.is_empty() {
            return Err(Status::invalid_argument("endpoints not set"));
        }
        if let Some(public_ip) = &request.public_ip {
            public_ip
                .to_addr()
                .map_err(|error| Status::invalid_argument(format!("invalid public IP: {error}")))?;
        }

        let machines = self
            .store
            .list_machines()
            .await
            .map_err(|error| Status::internal(format!("list machines: {error}")))?;
        let requested_management = network
            .management_ip
            .as_ref()
            .and_then(|address| address.to_addr().ok());
        let mut allocated = Vec::with_capacity(machines.len());
        for machine in &machines {
            if !request.name.is_empty() && machine.name == request.name {
                return Err(Status::already_exists(format!(
                    "machine with name {} already exists",
                    go_quote(&request.name)
                )));
            }
            let stored_network = machine
                .network
                .as_ref()
                .expect("stored machine network not set");
            if requested_management.as_ref().is_some_and(|requested| {
                stored_network
                    .management_ip
                    .as_ref()
                    .and_then(|address| address.to_addr().ok())
                    .as_ref()
                    == Some(requested)
            }) {
                let address = requested_management
                    .as_ref()
                    .expect("guarded by is_some")
                    .ip();
                return Err(Status::already_exists(format!(
                    "machine with management IP \"{address}\" already exists under the name {}",
                    go_quote(&machine.name)
                )));
            }
            if stored_network.public_key == network.public_key {
                let key = Secret::from(stored_network.public_key.clone()).to_hex_string();
                return Err(Status::already_exists(format!(
                    "machine with public key {} already exists under the name {}",
                    go_quote(&key),
                    go_quote(&machine.name)
                )));
            }
            let subnet = stored_network
                .subnet
                .as_ref()
                .expect("stored machine subnet not set");
            allocated.push(prefix_from_proto(subnet).expect("stored machine subnet is valid"));
        }

        let machine_id = new_machine_id()
            .map_err(|error| Status::internal(format!("generate machine ID: {error}")))?;
        let name = if request.name.is_empty() {
            new_random_machine_name()
                .map_err(|error| Status::internal(format!("generate machine name: {error}")))?
        } else {
            request.name
        };
        let management = match network.management_ip {
            Some(address) => address,
            None => {
                let address = management_ip(&Secret::from(network.public_key.clone()))
                    .map_err(|error| Status::internal(format!("derive management IP: {error}")))?;
                Ip::new(IpAddr::V6(address))
            }
        };
        let cluster_network = self.network().await?;
        let mut ipam = Ipam::with_allocated(cluster_network, allocated)
            .map_err(|error| Status::internal(format!("create IPAM manager: {error}")))?;
        let subnet = ipam
            .allocate_subnet_len(DEFAULT_SUBNET_BITS)
            .map_err(|error| Status::internal(format!("allocate subnet for machine: {error}")))?;
        let machine = MachineInfo {
            id: machine_id,
            name,
            network: Some(NetworkConfig {
                subnet: Some(prefix_to_proto(subnet)),
                management_ip: Some(management),
                endpoints: network.endpoints,
                public_key: network.public_key,
            }),
            public_ip: request.public_ip,
            ..MachineInfo::default()
        };
        self.store
            .create_machine(&machine)
            .await
            .map_err(|error| Status::internal(format!("create machine: {error}")))?;
        tracing::info!(
            id = machine.id,
            name = machine.name,
            subnet = %subnet,
            public_key = %Secret::from(machine.network.as_ref().expect("constructed above").public_key.clone()).to_hex_string(),
            "Machine added to the cluster."
        );
        Ok(AddMachineResponse {
            machine: Some(machine),
        })
    }

    async fn network(&self) -> Result<IpPrefix, Status> {
        let network = self
            .store
            .get_string(NETWORK_KEY)
            .await
            .map_err(|error| Status::internal(format!("get network from store: {error}")))?;
        IpPrefix::from_str(&network)
            .map_err(|error| Status::internal(format!("parse network prefix: {error}")))
    }

    fn check_ready(&self) -> Result<(), Status> {
        if self.ready.is_signaled() {
            Ok(())
        } else {
            Err(Status::unavailable(
                "machine is not ready to serve cluster requests",
            ))
        }
    }
    pub async fn member_rtts(&self) -> Result<Vec<MemberRttStats>, BoxError> {
        self.admin.member_rtts().await
    }

    async fn list_machines_inner(&self) -> Result<ListMachinesResponse, Status> {
        self.check_ready()?;
        let machines = self
            .store
            .list_machines()
            .await
            .map_err(|error| Status::internal(error.to_string()))?;
        let states =
            self.admin.membership_states().await.map_err(|error| {
                Status::internal(format!("get cluster membership states: {error}"))
            })?;
        let local_id = read_lock(&self.machine_id).clone();
        let machines = machines
            .into_iter()
            .map(|machine| {
                let management_ip = machine
                    .network
                    .as_ref()
                    .expect("stored machine network not set")
                    .management_ip
                    .as_ref()
                    .expect("stored machine management IP not set")
                    .to_addr()
                    .expect("stored machine management IP is valid");
                let state = states
                    .iter()
                    .find(|state| addresses_equal(&state.address.ip(), &management_ip))
                    .map_or(machine_member::MembershipState::Down, |state| {
                        match state.state {
                            MembershipState::Alive => machine_member::MembershipState::Up,
                            MembershipState::Suspect => machine_member::MembershipState::Suspect,
                            MembershipState::Down => machine_member::MembershipState::Down,
                        }
                    });
                let state = if machine.id == local_id {
                    machine_member::MembershipState::Up
                } else {
                    state
                };
                MachineMember {
                    machine: Some(machine),
                    state: state as i32,
                }
            })
            .collect();
        Ok(ListMachinesResponse { machines })
    }
    async fn stored_domain(&self) -> Result<StoredDomain, Status> {
        let json = match self.store.get_bytes(UNCLOUD_DNS_KEY).await {
            Ok(json) => json,
            Err(StoreError::KeyNotFound) => return Err(Status::not_found("domain not found")),
            Err(error) => {
                return Err(Status::internal(format!("get domain from store: {error}")));
            }
        };
        serde_json::from_slice(&json)
            .map_err(|error| Status::internal(format!("unmarshal domain: {error}")))
    }

    async fn reserve_domain_inner(&self, request: ReserveDomainRequest) -> Result<Domain, Status> {
        self.check_ready()?;
        if request.endpoint.is_empty() {
            return Err(Status::invalid_argument("API endpoint not set"));
        }
        match self.stored_domain().await {
            Ok(_) => return Err(Status::already_exists("domain already reserved")),
            Err(status) if status.code() == Code::NotFound => {}
            Err(status) => return Err(status),
        }
        let dns = self.dns.clone();
        let endpoint = request.endpoint;
        let endpoint_for_call = endpoint.clone();
        let (name, token) =
            tokio::task::spawn_blocking(move || dns.reserve_domain(&endpoint_for_call))
                .await
                .map_err(|error| Status::internal(format!("DNS reservation task failed: {error}")))?
                .map_err(|error| Status::internal(error.to_string()))?;
        let domain = StoredDomain {
            endpoint,
            name: name.clone(),
            token,
        };
        let json = encode_stored_domain(&domain);
        self.store
            .put_bytes(UNCLOUD_DNS_KEY, &json)
            .await
            .map_err(|error| Status::internal(format!("store reserved domain: {error}")))?;
        Ok(Domain { name })
    }

    async fn get_domain_inner(&self) -> Result<Domain, Status> {
        self.check_ready()?;
        self.stored_domain()
            .await
            .map(|domain| Domain { name: domain.name })
    }

    async fn release_domain_inner(&self) -> Result<Domain, Status> {
        self.check_ready()?;
        let domain = self.stored_domain().await?;
        self.store
            .delete(UNCLOUD_DNS_KEY)
            .await
            .map_err(|error| Status::internal(format!("delete domain from store: {error}")))?;
        Ok(Domain { name: domain.name })
    }

    async fn create_domain_records_inner(
        &self,
        request: CreateDomainRecordsRequest,
    ) -> Result<CreateDomainRecordsResponse, Status> {
        self.check_ready()?;
        let domain = self.stored_domain().await?;
        let records = request
            .records
            .iter()
            .map(map_record_request)
            .collect::<Vec<_>>();
        let dns = self.dns.clone();
        let responses = tokio::task::spawn_blocking(move || {
            dns.create_records(&domain.endpoint, &domain.name, &domain.token, &records)
        })
        .await
        .map_err(|error| Status::unknown(format!("DNS records task failed: {error}")))?
        .map_err(|error| Status::unknown(error.to_string()))?;
        Ok(CreateDomainRecordsResponse {
            records: responses.iter().map(map_record_response).collect(),
        })
    }
}

#[tonic::async_trait]
impl cluster_server::Cluster for Cluster {
    async fn add_machine(
        &self,
        request: Request<AddMachineRequest>,
    ) -> Result<Response<AddMachineResponse>, Status> {
        self.check_ready()?;
        self.add_machine_without_ready_check(request.into_inner())
            .await
            .map(Response::new)
    }

    async fn list_machines(
        &self,
        _request: Request<()>,
    ) -> Result<Response<ListMachinesResponse>, Status> {
        self.list_machines_inner().await.map(Response::new)
    }

    async fn remove_machine(
        &self,
        request: Request<RemoveMachineRequest>,
    ) -> Result<Response<()>, Status> {
        self.check_ready()?;
        let id = request.into_inner().id;
        if id.is_empty() {
            return Err(Status::invalid_argument("machine ID not set"));
        }
        if let Err(error) = self.store.delete_machine_containers(&id).await {
            tracing::error!(id, error = %error, "Failed to delete container records from the cluster store for the machine being removed.");
        }
        match self.store.delete_machine(&id).await {
            Ok(()) => {}
            Err(StoreError::MachineNotFound(_)) => {
                return Err(Status::not_found(format!("machine not found: {id}")));
            }
            Err(error) => {
                return Err(Status::internal(format!(
                    "delete machine from store: {error}"
                )));
            }
        }
        tracing::info!(id, "Machine removed from the cluster.");
        Ok(Response::new(()))
    }

    async fn reserve_domain(
        &self,
        request: Request<ReserveDomainRequest>,
    ) -> Result<Response<Domain>, Status> {
        self.reserve_domain_inner(request.into_inner())
            .await
            .map(Response::new)
    }

    async fn get_domain(&self, _request: Request<()>) -> Result<Response<Domain>, Status> {
        self.get_domain_inner().await.map(Response::new)
    }

    async fn release_domain(&self, _request: Request<()>) -> Result<Response<Domain>, Status> {
        self.release_domain_inner().await.map(Response::new)
    }

    async fn create_domain_records(
        &self,
        request: Request<CreateDomainRecordsRequest>,
    ) -> Result<Response<CreateDomainRecordsResponse>, Status> {
        self.create_domain_records_inner(request.into_inner())
            .await
            .map(Response::new)
    }
}

fn prefix_from_proto(
    prefix: &PbIpPrefix,
) -> Result<IpPrefix, ployz_internal_machine_api_pb::AddressError> {
    let (address, bits) = prefix.to_prefix()?;
    Ok(IpPrefix::new(address, bits).expect("protobuf helper validated prefix length"))
}

fn prefix_to_proto(prefix: IpPrefix) -> PbIpPrefix {
    PbIpPrefix::new(prefix.address(), prefix.bits())
}

fn addresses_equal(address: &IpAddr, management: &Address) -> bool {
    management.zone().is_empty() && *address == management.ip()
}

fn read_lock(lock: &RwLock<String>) -> std::sync::RwLockReadGuard<'_, String> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write_lock(lock: &RwLock<String>) -> std::sync::RwLockWriteGuard<'_, String> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn go_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\u{0007}' => quoted.push_str("\\a"),
            '\u{0008}' => quoted.push_str("\\b"),
            '\t' => quoted.push_str("\\t"),
            '\n' => quoted.push_str("\\n"),
            '\u{000b}' => quoted.push_str("\\v"),
            '\u{000c}' => quoted.push_str("\\f"),
            '\r' => quoted.push_str("\\r"),
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            character if character.is_ascii_control() => {
                use std::fmt::Write as _;
                write!(quoted, "\\x{:02x}", u32::from(character))
                    .expect("writing to String cannot fail");
            }
            '\u{007f}' => quoted.push_str("\\x7f"),
            '\u{2028}' => quoted.push_str("\\u2028"),
            '\u{2029}' => quoted.push_str("\\u2029"),
            character if !crate::go_printable::is_print(character) => {
                use std::fmt::Write as _;
                let codepoint = u32::from(character);
                if codepoint <= 0xffff {
                    write!(quoted, "\\u{codepoint:04x}").expect("writing to String cannot fail");
                } else {
                    write!(quoted, "\\U{codepoint:08x}").expect("writing to String cannot fail");
                }
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn rfc3339_utc(time: SystemTime) -> String {
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::from(duration.as_secs()),
        Err(error) => {
            let duration = error.duration();
            -i128::from(duration.as_secs()) - i128::from(duration.subsec_nanos() != 0)
        }
    };
    let days =
        i64::try_from(seconds.div_euclid(86_400)).expect("SystemTime's day range fits in i64");
    let day_seconds =
        u64::try_from(seconds.rem_euclid(86_400)).expect("seconds within a day fit in u64");
    let (year, month, day) = civil_date(days);
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_date(days_since_epoch: i64) -> (i64, u64, u64) {
    let days = days_since_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (
        year,
        u64::try_from(month).expect("civil month is positive"),
        u64::try_from(day).expect("civil day is positive"),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::Mutex;
    use std::time::Duration;

    use ployz_internal_dns::{
        CreateRecordsError, Error as DnsError, RecordRequest, RecordResponse,
    };
    use ployz_internal_machine_api_pb::{IpPort, dns_record};

    use super::*;
    use crate::DEFAULT_NETWORK;
    use ployz_internal_machine_api_pb::cluster_server::Cluster as _;

    #[derive(Clone, Default)]
    struct MemoryStore(Arc<Mutex<MemoryState>>);

    #[derive(Default)]
    struct MemoryState {
        values: BTreeMap<String, Vec<u8>>,
        machines: Vec<MachineInfo>,
        fail_container_delete: bool,
    }

    impl MemoryStore {
        fn value(&self, key: &str) -> Option<Vec<u8>> {
            self.0.lock().unwrap().values.get(key).cloned()
        }
    }

    #[tonic::async_trait]
    impl StoreAccess for MemoryStore {
        async fn put_string(&self, key: &str, value: &str) -> Result<(), StoreError> {
            self.0
                .lock()
                .unwrap()
                .values
                .insert(key.into(), value.as_bytes().to_vec());
            Ok(())
        }
        async fn put_bytes(&self, key: &str, value: &[u8]) -> Result<(), StoreError> {
            self.0
                .lock()
                .unwrap()
                .values
                .insert(key.into(), value.to_vec());
            Ok(())
        }
        async fn get_string(&self, key: &str) -> Result<String, StoreError> {
            let bytes = self.get_bytes(key).await?;
            String::from_utf8(bytes).map_err(|error| StoreError::InvalidData(error.to_string()))
        }
        async fn get_bytes(&self, key: &str) -> Result<Vec<u8>, StoreError> {
            self.value(key).ok_or(StoreError::KeyNotFound)
        }
        async fn delete(&self, key: &str) -> Result<(), StoreError> {
            self.0.lock().unwrap().values.remove(key);
            Ok(())
        }
        async fn list_machines(&self) -> Result<Vec<MachineInfo>, StoreError> {
            Ok(self.0.lock().unwrap().machines.clone())
        }
        async fn create_machine(&self, machine: &MachineInfo) -> Result<(), StoreError> {
            self.0.lock().unwrap().machines.push(machine.clone());
            Ok(())
        }
        async fn delete_machine(&self, id: &str) -> Result<(), StoreError> {
            let mut state = self.0.lock().unwrap();
            let Some(index) = state.machines.iter().position(|machine| machine.id == id) else {
                return Err(StoreError::MachineNotFound(id.into()));
            };
            state.machines.remove(index);
            Ok(())
        }
        async fn delete_machine_containers(&self, _id: &str) -> Result<(), StoreError> {
            if self.0.lock().unwrap().fail_container_delete {
                Err(StoreError::InvalidData("container cleanup failed".into()))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Clone, Default)]
    struct MemoryAdmin {
        states: Arc<Vec<ClusterMembershipState>>,
        rtts: Arc<Vec<MemberRttStats>>,
    }

    #[tonic::async_trait]
    impl AdminAccess for MemoryAdmin {
        async fn membership_states(&self) -> Result<Vec<ClusterMembershipState>, BoxError> {
            Ok((*self.states).clone())
        }
        async fn member_rtts(&self) -> Result<Vec<MemberRttStats>, BoxError> {
            Ok((*self.rtts).clone())
        }
    }

    #[derive(Clone, Default)]
    struct MemoryDns;

    impl DnsAccess for MemoryDns {
        fn reserve_domain(&self, _endpoint: &str) -> Result<(String, String), DnsError> {
            Ok(("example.uncloud.run".into(), "secret-token".into()))
        }
        fn create_records(
            &self,
            _endpoint: &str,
            _domain: &str,
            _token: &str,
            records: &[RecordRequest],
        ) -> Result<Vec<RecordResponse>, CreateRecordsError> {
            Ok(records
                .iter()
                .cloned()
                .map(|record| RecordResponse {
                    fqdn: format!("{}.example.uncloud.run", record.name),
                    record,
                })
                .collect())
        }
    }

    fn service(ready: bool) -> (Cluster, MemoryStore) {
        let ready_latch = Latch::new();
        if ready {
            ready_latch.signal();
        }
        let store = MemoryStore::default();
        let cluster = Cluster::with_dependencies(
            store.clone(),
            MemoryAdmin::default(),
            MemoryDns,
            Latch::new(),
            ready_latch,
        );
        (cluster, store)
    }

    fn endpoint() -> IpPort {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 51820)).into()
    }

    fn add_request(name: &str, key_byte: u8) -> AddMachineRequest {
        AddMachineRequest {
            name: name.into(),
            network: Some(NetworkConfig {
                endpoints: vec![endpoint()],
                public_key: vec![key_byte; 32],
                ..NetworkConfig::default()
            }),
            public_ip: None,
        }
    }

    #[test]
    fn utc_timestamp_formatter_matches_rfc3339_boundaries() {
        assert_eq!(rfc3339_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            rfc3339_utc(UNIX_EPOCH + Duration::from_secs(951_827_696)),
            "2000-02-29T12:34:56Z"
        );
        assert_eq!(
            rfc3339_utc(UNIX_EPOCH - Duration::from_millis(1)),
            "1969-12-31T23:59:59Z"
        );
    }

    #[test]
    fn quoted_error_values_match_go_string_quoting() {
        assert_eq!(go_quote("web\n\u{0001}雪"), "\"web\\n\\x01雪\"");
        assert_eq!(
            go_quote("web\u{2028}\u{2029}\u{200b}"),
            "\"web\\u2028\\u2029\\u200b\""
        );
        assert_eq!(go_quote("e\u{0301}\u{fe0f}"), "\"e\u{0301}\u{fe0f}\"");
    }

    #[tokio::test]
    async fn init_writes_network_then_utc_creation_time_and_rejects_reinit() {
        let initialized = Latch::new();
        let store = MemoryStore::default();
        let cluster = Cluster::with_dependencies(
            store.clone(),
            MemoryAdmin::default(),
            MemoryDns,
            initialized.clone(),
            Latch::new(),
        );
        cluster.init(DEFAULT_NETWORK).await.unwrap();
        assert_eq!(store.value(NETWORK_KEY).unwrap(), b"10.210.0.0/16");
        let created = String::from_utf8(store.value(CREATED_AT_KEY).unwrap()).unwrap();
        assert_eq!(created.len(), 20);
        assert!(created.ends_with('Z'));
        initialized.signal();
        assert!(matches!(
            cluster.init(DEFAULT_NETWORK).await,
            Err(ClusterInitError::AlreadyInitialised)
        ));
    }

    #[tokio::test]
    async fn readiness_is_checked_before_request_validation() {
        let (cluster, _) = service(false);
        let error = cluster
            .add_machine(Request::new(AddMachineRequest::default()))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::Unavailable);
        assert_eq!(
            error.message(),
            "machine is not ready to serve cluster requests"
        );
    }

    #[tokio::test]
    async fn add_machine_validates_and_allocates_first_subnet() {
        let (cluster, _) = service(true);
        cluster
            .store
            .put_string(NETWORK_KEY, "10.210.0.0/16")
            .await
            .unwrap();
        let response = cluster
            .add_machine_without_ready_check(add_request("web", 7))
            .await
            .unwrap();
        let machine = response.machine.unwrap();
        assert_eq!(machine.name, "web");
        assert_eq!(machine.id.len(), 32);
        let network = machine.network.unwrap();
        assert_eq!(
            prefix_from_proto(network.subnet.as_ref().unwrap()).unwrap(),
            "10.210.0.0/24".parse().unwrap()
        );
        assert_eq!(
            network.management_ip.unwrap().to_addr().unwrap().ip(),
            "fdcc:707:707:707:707:707:707:707"
                .parse::<IpAddr>()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn add_machine_rejects_duplicate_name_management_ip_and_key() {
        let (cluster, _) = service(true);
        cluster
            .store
            .put_string(NETWORK_KEY, "10.210.0.0/16")
            .await
            .unwrap();
        let first = cluster
            .add_machine_without_ready_check(add_request("web", 1))
            .await
            .unwrap()
            .machine
            .unwrap();

        let error = cluster
            .add_machine_without_ready_check(add_request("web", 2))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::AlreadyExists);
        assert_eq!(error.message(), "machine with name \"web\" already exists");

        let mut management_duplicate = add_request("db", 2);
        management_duplicate.network.as_mut().unwrap().management_ip =
            first.network.as_ref().unwrap().management_ip.clone();
        let error = cluster
            .add_machine_without_ready_check(management_duplicate)
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::AlreadyExists);
        assert!(error.message().contains("management IP"));

        let error = cluster
            .add_machine_without_ready_check(add_request("db", 1))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::AlreadyExists);
        assert!(error.message().contains("public key \"010101"));
    }

    #[tokio::test]
    async fn list_maps_membership_and_forces_local_machine_up() {
        let (mut cluster, _) = service(true);
        cluster
            .store
            .put_string(NETWORK_KEY, "10.210.0.0/16")
            .await
            .unwrap();
        let first = cluster
            .add_machine_without_ready_check(add_request("one", 1))
            .await
            .unwrap()
            .machine
            .unwrap();
        let second = cluster
            .add_machine_without_ready_check(add_request("two", 2))
            .await
            .unwrap()
            .machine
            .unwrap();
        let first_ip = first
            .network
            .as_ref()
            .unwrap()
            .management_ip
            .as_ref()
            .unwrap()
            .to_addr()
            .unwrap()
            .ip();
        cluster.admin = Arc::new(MemoryAdmin {
            states: Arc::new(vec![ClusterMembershipState {
                id: "corrosion-id".into(),
                address: SocketAddr::new(first_ip, 8787),
                state: MembershipState::Suspect,
                timestamp: ployz_internal_corrosion::NtpTimestamp::from_ntp64(0),
            }]),
            rtts: Arc::new(Vec::new()),
        });
        cluster.update_machine_id(second.id.clone());
        let response = cluster.list_machines_inner().await.unwrap();
        assert_eq!(
            response.machines[0].state,
            machine_member::MembershipState::Suspect as i32
        );
        assert_eq!(
            response.machines[1].state,
            machine_member::MembershipState::Up as i32
        );
    }

    #[tokio::test]
    async fn removal_swallow_container_cleanup_failure_but_not_machine_failure() {
        let (cluster, store) = service(true);
        cluster
            .store
            .put_string(NETWORK_KEY, "10.210.0.0/16")
            .await
            .unwrap();
        let machine = cluster
            .add_machine_without_ready_check(add_request("web", 1))
            .await
            .unwrap()
            .machine
            .unwrap();
        store.0.lock().unwrap().fail_container_delete = true;
        cluster
            .remove_machine(Request::new(RemoveMachineRequest {
                id: machine.id.clone(),
            }))
            .await
            .unwrap();
        let error = cluster
            .remove_machine(Request::new(RemoveMachineRequest { id: machine.id }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::NotFound);
    }

    #[tokio::test]
    async fn domain_lifecycle_uses_go_json_shape_and_maps_records() {
        let (cluster, store) = service(true);
        let domain = cluster
            .reserve_domain_inner(ReserveDomainRequest {
                endpoint: "https://dns.example/api".into(),
            })
            .await
            .unwrap();
        assert_eq!(domain.name, "example.uncloud.run");
        assert_eq!(store.value(UNCLOUD_DNS_KEY).unwrap(), br#"{"Endpoint":"https://dns.example/api","Name":"example.uncloud.run","Token":"secret-token"}"#);
        assert_eq!(cluster.get_domain_inner().await.unwrap(), domain);

        let response = cluster
            .create_domain_records_inner(CreateDomainRecordsRequest {
                records: vec![ployz_internal_machine_api_pb::DnsRecord {
                    name: "www".into(),
                    r#type: dns_record::RecordType::A as i32,
                    values: vec!["192.0.2.1".into()],
                }],
            })
            .await
            .unwrap();
        assert_eq!(response.records[0].name, "www.example.uncloud.run");
        assert_eq!(response.records[0].r#type, dns_record::RecordType::A as i32);
        assert_eq!(cluster.release_domain_inner().await.unwrap(), domain);
        assert_eq!(
            cluster.get_domain_inner().await.unwrap_err().code(),
            Code::NotFound
        );
    }
}
