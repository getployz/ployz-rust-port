use ployz_internal_machine_api_pb as pb;
use ployz_pkg_api::ServiceSpec;

use crate::constraint::{Constraint, constraints_from_spec};
use crate::{ClusterState, Machine, ScheduleError};

/// Applies service placement and named-volume constraints to a cluster snapshot.
#[derive(Debug)]
pub struct ServiceScheduler<'a> {
    state: &'a ClusterState,
    constraints: Vec<Box<dyn Constraint>>,
}

impl<'a> ServiceScheduler<'a> {
    #[must_use]
    pub fn new(state: &'a ClusterState, spec: &ServiceSpec) -> Self {
        let constraints = constraints_from_spec(spec);
        Self { state, constraints }
    }

    pub fn eligible_machines(&self) -> Result<Vec<&'a Machine>, ScheduleError> {
        let available = self
            .state
            .machines
            .iter()
            .filter(|machine| {
                self.constraints
                    .iter()
                    .all(|constraint| constraint.evaluate(machine))
            })
            .collect::<Vec<_>>();
        if available.is_empty() {
            Err(ScheduleError::NoEligibleMachines)
        } else {
            Ok(available)
        }
    }

    /// Retains the frozen package's explicit unsupported operation.
    pub fn schedule_container(&self) -> Result<Vec<pb::MachineInfo>, ScheduleError> {
        Err(ScheduleError::ContainerSchedulingUnsupported)
    }
}
