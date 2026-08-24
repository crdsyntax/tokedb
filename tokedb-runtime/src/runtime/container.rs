use serde::{Deserialize, Serialize};

use crate::runtime::lifecycle::ContainerState;
use crate::runtime::process::CommandSpec;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub command: CommandSpec,
    pub resources: ResourceLimits,
    pub volumes: Vec<VolumeMount>,
    pub ports: Vec<PortBinding>,
    pub state: ContainerState,
    pub created_at: u64,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    pub command: CommandSpec,
    pub resources: ResourceLimits,
    pub volumes: Vec<VolumeMount>,
    pub ports: Vec<PortBinding>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub memory_bytes: Option<u64>,
    pub cpu_quota: Option<f64>,
    pub pids_max: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMount {
    pub name: String,
    pub mount_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortBinding {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
}
