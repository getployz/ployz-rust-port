use std::fmt;

use ployz_internal_machine_api_pb as pb;
use ployz_pkg_api::{
    ApiError, DockerVolume, MachineClient, MachineFilter, VolumeClient, VolumeSpec,
};

/// The current and planned resources on every available cluster machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClusterState {
    pub machines: Vec<Machine>,
}

/// Resources relevant to placement on one machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Machine {
    pub info: pb::MachineInfo,
    pub volumes: Vec<DockerVolume>,
    pub scheduled_volumes: Vec<VolumeSpec>,
}

/// Client capabilities needed to inspect scheduler state.
pub trait Client: MachineClient + VolumeClient {}

impl<T> Client for T where T: MachineClient + VolumeClient {}

/// Failure while taking the two-query cluster snapshot used by the scheduler.
#[derive(Clone, Debug)]
pub enum InspectError {
    ListMachines(ApiError),
    ListVolumes(ApiError),
}

impl fmt::Display for InspectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ListMachines(error) => write!(formatter, "list machines: {error}"),
            Self::ListVolumes(error) => write!(formatter, "list volumes: {error}"),
        }
    }
}

impl std::error::Error for InspectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ListMachines(error) | Self::ListVolumes(error) => Some(error),
        }
    }
}

/// Inspects available machines followed by volumes, retaining the oracle's
/// non-atomic two-query snapshot behavior.
pub async fn inspect_cluster_state(client: &impl Client) -> Result<ClusterState, InspectError> {
    let members = client
        .list_machines(Some(MachineFilter {
            available: true,
            ..MachineFilter::default()
        }))
        .await
        .map_err(InspectError::ListMachines)?;
    let volumes = client
        .list_volumes(None)
        .await
        .map_err(InspectError::ListVolumes)?;

    let machines = members
        .0
        .into_iter()
        .map(|member| {
            // The Go implementation dereferences this generated pointer.
            let info = member.machine.expect("Machine not set");
            let machine_volumes = volumes
                .iter()
                .filter(|volume| volume.machine_id == info.id)
                .map(|volume| volume.volume.clone())
                .collect();
            Machine {
                info,
                volumes: machine_volumes,
                scheduled_volumes: Vec::new(),
            }
        })
        .collect();

    Ok(ClusterState { machines })
}

impl ClusterState {
    #[must_use]
    pub fn machine(&self, name_or_id: &str) -> Option<&Machine> {
        self.machines
            .iter()
            .find(|machine| machine.info.id == name_or_id || machine.info.name == name_or_id)
    }

    #[must_use]
    pub fn machine_mut(&mut self, name_or_id: &str) -> Option<&mut Machine> {
        self.machines
            .iter_mut()
            .find(|machine| machine.info.id == name_or_id || machine.info.name == name_or_id)
    }

    #[must_use]
    pub fn machine_name(&self, id: &str) -> Option<&str> {
        self.machine(id).map(|machine| machine.info.name.as_str())
    }
}
