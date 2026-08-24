#![cfg(feature = "iptables")]

use std::net::Ipv4Addr;

#[cfg(target_os = "linux")]
use crate::error::{Result, RuntimeError};
use crate::network::port::PortMap;

fn dnat_rule(
    protocol: &str,
    host_port: u16,
    container_ip: Ipv4Addr,
    container_port: u16,
) -> Vec<String> {
    vec![
        "-t".to_string(),
        "nat".to_string(),
        "-A".to_string(),
        "PREROUTING".to_string(),
        "-p".to_string(),
        protocol.to_string(),
        "--dport".to_string(),
        host_port.to_string(),
        "-j".to_string(),
        "DNAT".to_string(),
        "--to-destination".to_string(),
        format!("{container_ip}:{container_port}"),
    ]
}

fn forward_rule(protocol: &str, host_port: u16) -> Vec<String> {
    vec![
        "-A".to_string(),
        "FORWARD".to_string(),
        "-p".to_string(),
        protocol.to_string(),
        "--dport".to_string(),
        host_port.to_string(),
        "-j".to_string(),
        "ACCEPT".to_string(),
    ]
}

pub fn dnat_rules(container_ip: Ipv4Addr, maps: &[PortMap]) -> Vec<Vec<String>> {
    let mut rules = Vec::new();
    for map in maps {
        let protocol = match map.protocol {
            crate::network::port::PortProtocol::Tcp => "tcp",
            crate::network::port::PortProtocol::Udp => "udp",
        };
        rules.push(dnat_rule(
            protocol,
            map.host_port,
            container_ip,
            map.container_port,
        ));
        rules.push(forward_rule(protocol, map.host_port));
    }
    rules
}

#[cfg(target_os = "linux")]
pub fn apply(rules: &[Vec<String>]) -> Result<()> {
    for rule in rules {
        let output = std::process::Command::new("iptables")
            .args(rule)
            .output()
            .map_err(|err| RuntimeError::Process(format!("iptables: {err}")))?;
        if !output.status.success() {
            return Err(RuntimeError::Process(format!(
                "iptables {}: {}",
                rule.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dnat_rules_target_container_and_open_forward() {
        let rules = dnat_rules(Ipv4Addr::new(10, 20, 0, 5), &[PortMap::tcp(8080, 3306)]);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0][7], "8080");
        assert_eq!(rules[0][9], "DNAT");
        assert_eq!(rules[0][11], "10.20.0.5:3306");
        assert_eq!(rules[1][5], "8080");
        assert_eq!(rules[1][7], "ACCEPT");

        let udp_rules = dnat_rules(Ipv4Addr::new(10, 20, 0, 5), &[PortMap::udp(5353, 53)]);
        assert_eq!(udp_rules[0][5], "udp");
    }
}
