#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use crate::error::{Result, RuntimeError};

const NETLINK_ROUTE: i32 = 0;

const RTM_NEWLINK: u16 = 16;
const RTM_DELLINK: u16 = 17;
const RTM_GETLINK: u16 = 18;
const RTM_NEWADDR: u16 = 20;

const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;

const NLM_F_REQUEST: u16 = 0x0001;
const NLM_F_ACK: u16 = 0x0004;
const NLM_F_EXCL: u16 = 0x0200;
const NLM_F_CREATE: u16 = 0x0400;
const NLM_F_DUMP: u16 = 0x0300;

const IFLA_IFNAME: u16 = 3;
const IFLA_MASTER: u16 = 10;
const IFLA_LINKINFO: u16 = 18;
const IFLA_NET_NS_FD: u16 = 28;

const IFLA_INFO_KIND: u16 = 1;
const IFLA_INFO_DATA: u16 = 2;
const VETH_INFO_PEER: u16 = 1;

const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;

const IFF_UP: u32 = 0x0001;
const IFF_RUNNING: u32 = 0x0040;

const NLMSG_HDR_LEN: usize = 16;
const IFINFOMSG_LEN: usize = 16;
const IFADDRMSG_LEN: usize = 8;
const SOCKADDR_NL_LEN: usize = 12;

struct RtSocket {
    fd: OwnedFd,
    seq: u32,
}

fn nl_errno(errno: i32) -> RuntimeError {
    RuntimeError::Process(format!(
        "rtnetlink: {errno} ({})",
        io::Error::from_raw_os_error(errno)
    ))
}

fn pad4(buf: &mut Vec<u8>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

fn put_attr(buf: &mut Vec<u8>, ty: u16, data: &[u8]) {
    let len = 4 + data.len();
    let padded = (len + 3) & !3;
    buf.extend_from_slice(&(padded as u16).to_ne_bytes());
    buf.extend_from_slice(&ty.to_ne_bytes());
    buf.extend_from_slice(data);
    pad4(buf);
}

fn put_u32(buf: &mut Vec<u8>, ty: u16, value: u32) {
    put_attr(buf, ty, &value.to_ne_bytes());
}

fn cstr(name: &str) -> Vec<u8> {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

fn ifinfomsg_index(index: i32, flags: u32, change: u32) -> Vec<u8> {
    let mut msg = Vec::with_capacity(IFINFOMSG_LEN);
    msg.push(0); // family: AF_UNSPEC
    msg.push(0); // pad
    msg.extend_from_slice(&0u16.to_ne_bytes()); // ifi_type
    msg.extend_from_slice(&index.to_ne_bytes());
    msg.extend_from_slice(&flags.to_ne_bytes());
    msg.extend_from_slice(&change.to_ne_bytes());
    msg
}

fn ifinfomsg(flags: u32, change: u32) -> Vec<u8> {
    ifinfomsg_index(0, flags, change)
}

fn ifaddrmsg(family: u8, prefixlen: u8, index: u32) -> Vec<u8> {
    let mut msg = Vec::with_capacity(IFADDRMSG_LEN);
    msg.push(family);
    msg.push(prefixlen);
    msg.push(0); // flags
    msg.push(0); // scope: RT_SCOPE_UNIVERSE
    msg.extend_from_slice(&index.to_ne_bytes());
    msg
}

fn build_message(nl_type: u16, flags: u16, seq: u32, payload: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(NLMSG_HDR_LEN + payload.len());
    msg.extend_from_slice(&((NLMSG_HDR_LEN + payload.len()) as u32).to_ne_bytes());
    msg.extend_from_slice(&nl_type.to_ne_bytes());
    msg.extend_from_slice(&flags.to_ne_bytes());
    msg.extend_from_slice(&seq.to_ne_bytes());
    msg.extend_from_slice(&0u32.to_ne_bytes()); // nl_pid
    msg.extend_from_slice(payload);
    pad4(&mut msg);
    msg
}

fn netlink_sockaddr() -> [u8; SOCKADDR_NL_LEN] {
    let mut addr = [0u8; SOCKADDR_NL_LEN];
    addr[0..2].copy_from_slice(&(libc::AF_NETLINK as u16).to_ne_bytes());
    addr
}

impl RtSocket {
    fn open() -> Result<RtSocket> {
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                NETLINK_ROUTE,
            )
        };
        if fd < 0 {
            return Err(nl_errno(
                io::Error::last_os_error().raw_os_error().unwrap_or(0),
            ));
        }
        let sockaddr = netlink_sockaddr();
        let rc = unsafe {
            libc::bind(
                fd,
                sockaddr.as_ptr().cast::<libc::sockaddr>(),
                SOCKADDR_NL_LEN as u32,
            )
        };
        if rc < 0 {
            let err = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(nl_errno(err.raw_os_error().unwrap_or(0)));
        }
        Ok(RtSocket {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
            seq: 0,
        })
    }

    fn send(&mut self, nl_type: u16, flags: u16, payload: &[u8]) -> Result<()> {
        self.seq += 1;
        let msg = build_message(nl_type, flags, self.seq, payload);
        let dst = netlink_sockaddr();
        let sent = unsafe {
            libc::sendto(
                self.fd.as_raw_fd(),
                msg.as_ptr().cast(),
                msg.len(),
                0,
                dst.as_ptr().cast::<libc::sockaddr>(),
                SOCKADDR_NL_LEN as u32,
            )
        };
        if sent < 0 {
            return Err(nl_errno(
                io::Error::last_os_error().raw_os_error().unwrap_or(0),
            ));
        }
        Ok(())
    }

    fn recv_ack(&self) -> Result<Option<i32>> {
        loop {
            let mut buf = [0u8; 8192];
            let n =
                unsafe { libc::recv(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(nl_errno(err.raw_os_error().unwrap_or(0)));
            }
            let n = n as usize;
            let mut off = 0;
            while off + NLMSG_HDR_LEN <= n {
                let len = u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
                if len < NLMSG_HDR_LEN || off + len > n {
                    break;
                }
                let nl_type = u16::from_ne_bytes(buf[off + 4..off + 6].try_into().unwrap());
                let seq = u32::from_ne_bytes(buf[off + 8..off + 12].try_into().unwrap());
                let payload = &buf[off + NLMSG_HDR_LEN..off + len];
                if seq != self.seq {
                    off += (len + 3) & !3;
                    continue;
                }
                if nl_type == NLMSG_ERROR {
                    if payload.len() < 4 {
                        return Err(RuntimeError::Process(
                            "rtnetlink: truncated NLMSG_ERROR".into(),
                        ));
                    }
                    let error = i32::from_ne_bytes(payload[..4].try_into().unwrap());
                    return match error {
                        0 => Ok(None),
                        errno => Ok(Some(-errno)),
                    };
                }
                off += (len + 3) & !3;
            }
        }
    }

    fn dump(&mut self, nl_type: u16, payload: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.seq += 1;
        let msg = build_message(nl_type, NLM_F_REQUEST | NLM_F_DUMP, self.seq, payload);
        let dst = netlink_sockaddr();
        let sent = unsafe {
            libc::sendto(
                self.fd.as_raw_fd(),
                msg.as_ptr().cast(),
                msg.len(),
                0,
                dst.as_ptr().cast::<libc::sockaddr>(),
                SOCKADDR_NL_LEN as u32,
            )
        };
        if sent < 0 {
            return Err(nl_errno(
                io::Error::last_os_error().raw_os_error().unwrap_or(0),
            ));
        }
        let mut out = Vec::new();
        let mut finished = false;
        while !finished {
            let mut buf = [0u8; 8192];
            let n =
                unsafe { libc::recv(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(nl_errno(err.raw_os_error().unwrap_or(0)));
            }
            let n = n as usize;
            let mut off = 0;
            while off + NLMSG_HDR_LEN <= n {
                let len = u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
                if len < NLMSG_HDR_LEN || off + len > n {
                    break;
                }
                let nl_type = u16::from_ne_bytes(buf[off + 4..off + 6].try_into().unwrap());
                let payload = &buf[off + NLMSG_HDR_LEN..off + len];
                if nl_type == NLMSG_DONE || nl_type == NLMSG_ERROR {
                    if nl_type == NLMSG_ERROR && payload.len() >= 4 {
                        let error = i32::from_ne_bytes(payload[..4].try_into().unwrap());
                        if error != 0 {
                            return Err(nl_errno(-error));
                        }
                    }
                    finished = true;
                } else {
                    out.push(payload.to_vec());
                }
                off += (len + 3) & !3;
            }
        }
        Ok(out)
    }
}

fn netlink_error(errno: Option<i32>) -> Result<()> {
    match errno {
        Some(errno) => Err(nl_errno(errno)),
        None => Ok(()),
    }
}

pub fn create_veth_pair(host_name: &str, peer_name: &str) -> Result<()> {
    let mut socket = RtSocket::open()?;
    let mut peer = ifinfomsg(0, 0);
    put_attr(&mut peer, IFLA_IFNAME, &cstr(peer_name));
    let mut info_data = Vec::new();
    put_attr(&mut info_data, VETH_INFO_PEER, &peer);
    let mut link_info = Vec::new();
    put_attr(&mut link_info, IFLA_INFO_KIND, &cstr("veth"));
    put_attr(&mut link_info, IFLA_INFO_DATA, &info_data);

    let mut payload = ifinfomsg(0, 0);
    put_attr(&mut payload, IFLA_IFNAME, &cstr(host_name));
    put_attr(&mut payload, IFLA_LINKINFO, &link_info);
    socket.send(
        RTM_NEWLINK,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
        &payload,
    )?;
    netlink_error(socket.recv_ack()?)
}

pub fn create_bridge(name: &str) -> Result<()> {
    let mut socket = RtSocket::open()?;
    let mut link_info = Vec::new();
    put_attr(&mut link_info, IFLA_INFO_KIND, &cstr("bridge"));
    let mut payload = ifinfomsg(0, 0);
    put_attr(&mut payload, IFLA_IFNAME, &cstr(name));
    put_attr(&mut payload, IFLA_LINKINFO, &link_info);
    socket.send(
        RTM_NEWLINK,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
        &payload,
    )?;
    netlink_error(socket.recv_ack()?)
}

pub fn delete_link(name: &str) -> Result<()> {
    let mut socket = RtSocket::open()?;
    let mut payload = ifinfomsg(0, 0);
    put_attr(&mut payload, IFLA_IFNAME, &cstr(name));
    socket.send(RTM_DELLINK, NLM_F_REQUEST | NLM_F_ACK, &payload)?;
    netlink_error(socket.recv_ack()?)
}

pub fn link_index(name: &str) -> Result<u32> {
    let mut socket = RtSocket::open()?;
    let payload = ifinfomsg(0, 0);
    for msg in socket.dump(RTM_GETLINK, &payload)? {
        if msg.len() < IFINFOMSG_LEN {
            continue;
        }
        let index = i32::from_ne_bytes(msg[4..8].try_into().unwrap());
        let mut off = IFINFOMSG_LEN;
        while off + 4 <= msg.len() {
            let alen = u16::from_ne_bytes(msg[off..off + 2].try_into().unwrap()) as usize;
            let atype = u16::from_ne_bytes(msg[off + 2..off + 4].try_into().unwrap());
            if alen < 4 || off + alen > msg.len() {
                break;
            }
            if atype == IFLA_IFNAME {
                let value = &msg[off + 4..off + alen - 1];
                if value == name.as_bytes() {
                    return Ok(index as u32);
                }
            }
            off += (alen + 3) & !3;
        }
    }
    Err(RuntimeError::Process(format!(
        "rtnetlink: link `{name}` not found"
    )))
}

pub fn set_link_up_by_name(name: &str) -> Result<()> {
    let mut socket = RtSocket::open()?;
    let mut payload = ifinfomsg_index(0, IFF_UP | IFF_RUNNING, IFF_UP | IFF_RUNNING);
    put_attr(&mut payload, IFLA_IFNAME, &cstr(name));
    socket.send(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, &payload)?;
    netlink_error(socket.recv_ack()?)
}

pub fn rename_link_by_index(index: u32, new_name: &str) -> Result<()> {
    let mut socket = RtSocket::open()?;
    let mut payload = ifinfomsg_index(index as i32, 0, 0);
    put_attr(&mut payload, IFLA_IFNAME, &cstr(new_name));
    socket.send(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, &payload)?;
    netlink_error(socket.recv_ack()?)
}

pub fn set_link_master(host_name: &str, master_index: u32) -> Result<()> {
    let mut socket = RtSocket::open()?;
    let mut payload = ifinfomsg(0, 0);
    put_attr(&mut payload, IFLA_IFNAME, &cstr(host_name));
    put_u32(&mut payload, IFLA_MASTER, master_index);
    socket.send(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, &payload)?;
    netlink_error(socket.recv_ack()?)
}

pub fn move_link_to_netns(host_name: &str, netns_fd: &File) -> Result<()> {
    let mut socket = RtSocket::open()?;
    let mut payload = ifinfomsg(0, 0);
    put_attr(&mut payload, IFLA_IFNAME, &cstr(host_name));
    put_u32(&mut payload, IFLA_NET_NS_FD, netns_fd.as_raw_fd() as u32);
    socket.send(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, &payload)?;
    netlink_error(socket.recv_ack()?)
}

pub fn add_addr4(index: u32, prefixlen: u8, ip: [u8; 4]) -> Result<()> {
    let mut socket = RtSocket::open()?;
    let mut payload = ifaddrmsg(2, prefixlen, index);
    put_attr(&mut payload, IFA_LOCAL, &ip);
    put_attr(&mut payload, IFA_ADDRESS, &ip);
    socket.send(
        RTM_NEWADDR,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
        &payload,
    )?;
    match socket.recv_ack()? {
        None => Ok(()),
        Some(error) if error == libc::EEXIST => Ok(()),
        Some(error) => Err(nl_errno(error)),
    }
}
