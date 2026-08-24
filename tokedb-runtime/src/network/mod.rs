pub mod bridge;
#[cfg(target_os = "linux")]
pub mod namespace;
#[cfg(target_os = "linux")]
pub mod netlink;
pub mod port;

#[cfg(feature = "iptables")]
pub mod iptables;

pub use bridge::{
    bridge_gateway, container_ipv4, is_in_bridge_subnet, veth_host_name, BRIDGE_PREFIX,
    BRIDGE_SUBNET, DEFAULT_BRIDGE_NAME,
};
pub use port::{validate_port_maps, PortMap, PortProtocol};

#[cfg(target_os = "linux")]
pub use namespace::attach_container;

#[cfg(target_os = "linux")]
pub use bridge::{delete_bridge, ensure_bridge};

#[cfg(all(target_os = "linux", feature = "iptables"))]
pub use iptables::{apply as apply_iptables, dnat_rules};
