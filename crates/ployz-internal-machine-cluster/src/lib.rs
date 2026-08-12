//! Cluster membership, address allocation, and public DNS coordination.

mod dns;
mod go_printable;
mod ipam;
mod machine;
mod service;

pub use ipam::{DEFAULT_NETWORK, DEFAULT_SUBNET_BITS, IpPrefix, IpPrefixError, Ipam};
pub use machine::{
    MachineNameError, default_machine_name, machine_name_from_hostname, new_machine_id,
    new_random_machine_name,
};
pub use service::{Cluster, ClusterInitError, Latch};
