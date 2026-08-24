use std::net::Ipv4Addr;

use sha2::{Digest, Sha256};

#[cfg(target_os = "linux")]
use crate::error::{Result, RuntimeError};

pub const DEFAULT_BRIDGE_NAME: &str = "db0";
pub const BRIDGE_SUBNET: Ipv4Addr = Ipv4Addr::new(10, 20, 0, 0);
pub const BRIDGE_PREFIX: u8 = 24;

pub fn bridge_gateway() -> Ipv4Addr {
    Ipv4Addr::new(
        BRIDGE_SUBNET.octets()[0],
        BRIDGE_SUBNET.octets()[1],
        BRIDGE_SUBNET.octets()[2],
        1,
    )
}

pub fn container_ipv4(id: &str) -> Ipv4Addr {
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    let digest = hasher.finalize();
    let index = (u16::from(digest[0]) << 8 | u16::from(digest[1])) % 250 + 2;
    let [a, b, c, _] = BRIDGE_SUBNET.octets();
    Ipv4Addr::new(a, b, c, index as u8)
}

pub fn veth_host_name(ip: Ipv4Addr) -> String {
    let [_, _, _, host] = ip.octets();
    format!("ve{:02x}{:02x}{:02x}", ip.octets()[1], ip.octets()[2], host)
}

pub fn is_in_bridge_subnet(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = BRIDGE_SUBNET.octets();
    let octets = ip.octets();
    octets[0] == a && octets[1] == b && octets[2] == c
}

#[cfg(target_os = "linux")]
pub fn ensure_bridge(name: &str) -> Result<()> {
    use crate::network::netlink;

    match netlink::link_index(name) {
        Ok(_) => Ok(()),
        Err(RuntimeError::Process(ref message)) if message.contains("not found") => {
            netlink::create_bridge(name)?;
            netlink::set_link_up_by_name(name)?;
            netlink::add_addr4(
                netlink::link_index(name)?,
                BRIDGE_PREFIX,
                bridge_gateway().octets(),
            )?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

#[cfg(target_os = "linux")]
pub fn delete_bridge(name: &str) -> Result<()> {
    crate::network::netlink::delete_link(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_ip_is_deterministic_and_in_subnet() {
        let first = container_ipv4("c1");
        let second = container_ipv4("c1");
        assert_eq!(first, second);
        assert!(is_in_bridge_subnet(first));
        assert!(first.octets()[3] >= 2 && first.octets()[3] <= 251);
        assert_ne!(container_ipv4("c1"), container_ipv4("c2"));
    }

    #[test]
    fn gateway_is_network_first_address() {
        assert_eq!(bridge_gateway(), Ipv4Addr::new(10, 20, 0, 1));
        assert!(is_in_bridge_subnet(bridge_gateway()));
    }

    #[test]
    fn host_veth_name_is_short_and_stable() {
        let name = veth_host_name(container_ipv4("abc"));
        assert!(name.len() <= 15);
        assert!(!name.contains('.'));
        assert_eq!(veth_host_name(container_ipv4("abc")), name);
    }

    #[test]
    fn subnet_check_rejects_foreign_addresses() {
        assert!(!is_in_bridge_subnet(Ipv4Addr::new(192, 168, 1, 5)));
        assert!(!is_in_bridge_subnet(Ipv4Addr::new(10, 21, 0, 4)));
    }
}
