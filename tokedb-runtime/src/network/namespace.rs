#![cfg(target_os = "linux")]

use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::{Result, RuntimeError};
use crate::network::bridge::{container_ipv4, veth_host_name};
use crate::network::netlink;

pub fn attach_container(bridge: &str, host_pid: u32, id: &str) -> Result<String> {
    let ip = container_ipv4(id);
    let veth = veth_host_name(ip);
    let peer = format!("pe{}", &veth[2..]);

    netlink::create_veth_pair(&veth, &peer)?;
    let outcome = attach_impl(bridge, host_pid, &veth, &peer, &ip);
    if outcome.is_err() {
        // Don't leak the host veth when the attach failed midway.
        let _ = netlink::delete_link(&veth);
    }
    outcome
}

fn attach_impl(
    bridge: &str,
    host_pid: u32,
    veth: &str,
    peer: &str,
    ip: &std::net::Ipv4Addr,
) -> Result<String> {
    let bridge_index = netlink::link_index(bridge)?;
    let peer_index = netlink::link_index(peer)?;

    let netns = open_netns_fd(host_pid)?;
    netlink::move_link_to_netns(peer, &netns)?;
    netlink::set_link_up_by_name(veth)?;
    netlink::set_link_master(veth, bridge_index)?;

    let ip_octets = ip.octets();
    with_netns(&netns, move || {
        netlink::rename_link_by_index(peer_index, "eth0")?;
        netlink::set_link_up_by_name("eth0")?;
        netlink::add_addr4(
            netlink::link_index("eth0")?,
            crate::network::bridge::BRIDGE_PREFIX,
            ip_octets,
        )
    })?;

    Ok(veth.to_string())
}

pub fn detach_container(host_veth: &str) -> Result<()> {
    netlink::delete_link(host_veth)
}

pub fn open_netns_fd(host_pid: u32) -> Result<File> {
    let path = PathBuf::from(format!("/proc/{host_pid}/ns/net"));
    let mut attempts = 0;
    loop {
        attempts += 1;
        match fs::File::open(&path) {
            Ok(file) => return Ok(file),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && attempts < 50 => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(err) => {
                return Err(RuntimeError::Io {
                    path: path.display().to_string(),
                    message: err.to_string(),
                });
            }
        }
    }
}

pub fn with_netns(
    netns: &File,
    operation: impl FnOnce() -> Result<()> + Send + 'static,
) -> Result<()> {
    use nix::sched::{setns, CloneFlags};
    use std::os::fd::AsFd;

    let netns = Arc::new(netns.try_clone()?);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = setns(netns.as_fd(), CloneFlags::CLONE_NEWNET)
            .map_err(RuntimeError::from)
            .and_then(|_| operation());
        let _ = tx.send(result);
    });
    rx.recv()
        .map_err(|err| RuntimeError::Process(format!("netns helper thread: {err}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnet_constants_are_consistent() {
        assert_eq!(
            container_ipv4("x").octets()[..3],
            crate::network::bridge::BRIDGE_SUBNET.octets()[..3]
        );
        assert_eq!(veth_host_name(container_ipv4("x")).len(), 8);
    }
}
