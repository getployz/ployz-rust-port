use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use ployz_internal_machine_api_pb as pb;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{
    CreateContainerResponse, ExecOptions, MachineFilter, MachineImage, MachineMembersList,
    MachineRemoteImage, MachineServiceContainer, MachineVolume, Result, RunServiceResponse,
    Service, ServiceLogEntry, ServiceLogsOptions, ServiceSpec, VolumeFilter,
    WaitContainerHealthyOptions,
};

pub type ClientFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
pub type LogReceiver = mpsc::Receiver<ServiceLogEntry>;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct StopOptions {
    pub signal: String,
    pub timeout_seconds: Option<i32>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct RemoveOptions {
    pub remove_volumes: bool,
    pub remove_links: bool,
    pub force: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct CreateVolumeOptions {
    pub name: String,
    pub driver: String,
    pub driver_options: BTreeMap<String, String>,
    pub labels: BTreeMap<String, String>,
}

pub trait ContainerClient: Send + Sync {
    fn create_container<'a>(
        &'a self,
        service_id: &'a str,
        spec: ServiceSpec,
        machine_id: &'a str,
    ) -> ClientFuture<'a, CreateContainerResponse>;

    fn create_pre_deploy_hook_container<'a>(
        &'a self,
        service_id: &'a str,
        spec: ServiceSpec,
        machine_id: &'a str,
    ) -> ClientFuture<'a, CreateContainerResponse>;

    fn exec_container<'a>(
        &'a self,
        service_name_or_id: &'a str,
        container_name_or_id: &'a str,
        config: ExecOptions,
    ) -> ClientFuture<'a, i32>;

    fn inspect_container<'a>(
        &'a self,
        service_name_or_id: &'a str,
        container_name_or_id: &'a str,
    ) -> ClientFuture<'a, MachineServiceContainer>;

    fn start_container<'a>(
        &'a self,
        service_name_or_id: &'a str,
        container_name_or_id: &'a str,
    ) -> ClientFuture<'a, ()>;

    fn stop_container<'a>(
        &'a self,
        service_name_or_id: &'a str,
        container_name_or_id: &'a str,
        options: StopOptions,
    ) -> ClientFuture<'a, ()>;

    fn remove_container<'a>(
        &'a self,
        service_name_or_id: &'a str,
        container_name_or_id: &'a str,
        options: RemoveOptions,
    ) -> ClientFuture<'a, ()>;

    fn wait_container_healthy<'a>(
        &'a self,
        service_name_or_id: &'a str,
        container_name_or_id: &'a str,
        options: WaitContainerHealthyOptions,
    ) -> ClientFuture<'a, ()>;
}

pub trait DnsClient: Send + Sync {
    fn get_domain(&self) -> ClientFuture<'_, String>;
}

pub trait ImageClient: Send + Sync {
    fn inspect_image<'a>(&'a self, id: &'a str) -> ClientFuture<'a, Vec<MachineImage>>;
    fn inspect_remote_image<'a>(&'a self, id: &'a str)
    -> ClientFuture<'a, Vec<MachineRemoteImage>>;
}

pub trait LogsClient: Send + Sync {
    fn service_logs<'a>(
        &'a self,
        service_name_or_id: &'a str,
        options: ServiceLogsOptions,
    ) -> ClientFuture<'a, (Service, LogReceiver)>;

    fn machine_logs<'a>(
        &'a self,
        unit: &'a str,
        options: ServiceLogsOptions,
    ) -> ClientFuture<'a, LogReceiver>;
}

pub trait MachineClient: Send + Sync {
    fn inspect_machine<'a>(&'a self, id: &'a str) -> ClientFuture<'a, pb::MachineMember>;
    fn list_machines<'a>(
        &'a self,
        filter: Option<MachineFilter>,
    ) -> ClientFuture<'a, MachineMembersList>;
    fn update_machine<'a>(
        &'a self,
        name_or_id: &'a str,
        request: pb::UpdateMachineRequest,
    ) -> ClientFuture<'a, pb::MachineInfo>;
    fn rename_machine<'a>(
        &'a self,
        name_or_id: &'a str,
        new_name: &'a str,
    ) -> ClientFuture<'a, pb::MachineInfo>;
}

pub trait ServiceClient: Send + Sync {
    fn run_service(&self, spec: ServiceSpec) -> ClientFuture<'_, RunServiceResponse>;
    fn inspect_service<'a>(&'a self, id: &'a str) -> ClientFuture<'a, Service>;
    fn remove_service<'a>(&'a self, id: &'a str) -> ClientFuture<'a, ()>;
    fn stop_service<'a>(&'a self, id: &'a str, options: StopOptions) -> ClientFuture<'a, ()>;
    fn start_service<'a>(&'a self, id: &'a str) -> ClientFuture<'a, ()>;
}

pub trait VolumeClient: Send + Sync {
    fn create_volume<'a>(
        &'a self,
        machine_name_or_id: &'a str,
        options: CreateVolumeOptions,
    ) -> ClientFuture<'a, MachineVolume>;
    fn list_volumes<'a>(
        &'a self,
        filter: Option<VolumeFilter>,
    ) -> ClientFuture<'a, Vec<MachineVolume>>;
    fn remove_volume<'a>(
        &'a self,
        machine_name_or_id: &'a str,
        volume_name: &'a str,
        force: bool,
    ) -> ClientFuture<'a, ()>;
}

pub trait Client:
    ContainerClient
    + DnsClient
    + ImageClient
    + LogsClient
    + MachineClient
    + ServiceClient
    + VolumeClient
{
}

impl<T> Client for T where
    T: ContainerClient
        + DnsClient
        + ImageClient
        + LogsClient
        + MachineClient
        + ServiceClient
        + VolumeClient
{
}
