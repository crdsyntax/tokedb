use std::collections::HashSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Result, RuntimeError};
#[cfg(target_os = "linux")]
use std::net::Ipv4Addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMap {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: PortProtocol,
}

impl PortMap {
    pub fn tcp(host_port: u16, container_port: u16) -> PortMap {
        PortMap {
            host_port,
            container_port,
            protocol: PortProtocol::Tcp,
        }
    }

    pub fn udp(host_port: u16, container_port: u16) -> PortMap {
        PortMap {
            host_port,
            container_port,
            protocol: PortProtocol::Udp,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.host_port == 0 || self.container_port == 0 {
            return Err(RuntimeError::InvalidConfig(format!(
                "port mapping requires non-zero ports: {}:{} -> {}:{}",
                self.protocol_name(),
                self.host_port,
                self.protocol_name(),
                self.container_port
            )));
        }
        Ok(())
    }

    fn protocol_name(&self) -> &'static str {
        match self.protocol {
            PortProtocol::Tcp => "tcp",
            PortProtocol::Udp => "udp",
        }
    }
}

pub fn validate_port_maps(maps: &[PortMap]) -> Result<()> {
    let mut host_ports = HashSet::new();
    for map in maps {
        map.validate()?;
        if !host_ports.insert(map.host_port) {
            return Err(RuntimeError::InvalidConfig(format!(
                "duplicate host port in mapping: {}",
                map.host_port
            )));
        }
    }
    Ok(())
}

pub fn retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(200u64.saturating_mul(1u64 << attempt.min(4)))
}

pub const MAX_CONNECT_ATTEMPTS: u32 = 5;

#[cfg(target_os = "linux")]
pub struct ProxyHandle {
    pub map: PortMap,
    task: tokio::task::JoinHandle<()>,
    runtime: Option<tokio::runtime::Runtime>,
}

#[cfg(target_os = "linux")]
impl ProxyHandle {
    pub fn cancel(&self) {
        self.task.abort();
    }
}

#[cfg(target_os = "linux")]
impl Drop for ProxyHandle {
    fn drop(&mut self) {
        self.task.abort();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_millis(500));
        }
    }
}

#[cfg(target_os = "linux")]
pub fn spawn_port_proxies(maps: &[PortMap], container_ip: Ipv4Addr) -> Result<Vec<ProxyHandle>> {
    validate_port_maps(maps)?;
    let mut handles = Vec::with_capacity(maps.len());
    for map in maps {
        let map = map.clone();
        let task_map = map.clone();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|err| RuntimeError::Process(format!("tokio runtime: {err}")))?;
        let task = runtime.spawn(async move {
            match task_map.protocol {
                PortProtocol::Tcp => proxy_tcp(task_map, container_ip).await,
                PortProtocol::Udp => proxy_udp(task_map, container_ip).await,
            }
        });
        tracing::info!(
            host_port = map.host_port,
            container_port = map.container_port,
            protocol = %map.protocol_name(),
            %container_ip,
            "port mapping started"
        );
        handles.push(ProxyHandle {
            map: map.clone(),
            task,
            runtime: Some(runtime),
        });
    }
    Ok(handles)
}

#[cfg(target_os = "linux")]
async fn proxy_tcp(map: PortMap, container_ip: Ipv4Addr) {
    use tokio::io::copy_bidirectional;
    use tokio::net::TcpListener;

    let listener = match TcpListener::bind(("0.0.0.0", map.host_port)).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!(host_port = map.host_port, error = %err, "tcp proxy bind failed");
            return;
        }
    };
    loop {
        match listener.accept().await {
            Ok((mut client, client_addr)) => {
                tracing::debug!(
                    host_port = map.host_port,
                    %client_addr,
                    "tcp proxy accepted connection"
                );
                tokio::spawn(async move {
                    let mut upstream =
                        match connect_with_retries(container_ip, map.container_port).await {
                            Ok(stream) => stream,
                            Err(err) => {
                                tracing::warn!(
                                    container_port = map.container_port,
                                    %container_ip,
                                    error = %err,
                                    "tcp proxy upstream connect failed"
                                );
                                return;
                            }
                        };
                    let _ = copy_bidirectional(&mut client, &mut upstream).await;
                });
            }
            Err(err) => {
                tracing::warn!(host_port = map.host_port, error = %err, "tcp proxy accept failed");
            }
        }
    }
}

#[cfg(target_os = "linux")]
async fn connect_with_retries(ip: Ipv4Addr, port: u16) -> std::io::Result<tokio::net::TcpStream> {
    use tokio::net::TcpStream;

    let mut last_error = None;
    for attempt in 1..=MAX_CONNECT_ATTEMPTS {
        match TcpStream::connect((ip, port)).await {
            Ok(stream) => return Ok(stream),
            Err(err) => {
                last_error = Some(err);
                tokio::time::sleep(retry_delay(attempt)).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("connect failed without error")))
}

#[cfg(target_os = "linux")]
async fn proxy_udp(map: PortMap, container_ip: Ipv4Addr) {
    use std::collections::HashMap;
    use tokio::net::UdpSocket;
    use tokio::sync::mpsc;

    let host_sock = match UdpSocket::bind(("0.0.0.0", map.host_port)).await {
        Ok(sock) => sock,
        Err(err) => {
            tracing::error!(host_port = map.host_port, error = %err, "udp proxy bind failed");
            return;
        }
    };
    let host_sock = std::sync::Arc::new(host_sock);

    let mut sinks: HashMap<
        std::net::SocketAddr,
        mpsc::UnboundedSender<(Vec<u8>, std::net::SocketAddr)>,
    > = HashMap::new();

    loop {
        let mut buf = vec![0u8; 65535];
        match host_sock.recv_from(&mut buf).await {
            Ok((len, client_addr)) => {
                let data = buf[..len].to_vec();
                let sink = sinks
                    .entry(client_addr)
                    .or_insert_with(|| {
                        let (tx, mut rx) = mpsc::unbounded_channel::<(Vec<u8>, std::net::SocketAddr)>();
                        let host_sock = host_sock.clone();
                        tokio::spawn(async move {
                            let upstream = match UdpSocket::bind("0.0.0.0:0").await {
                                Ok(sock) => sock,
                                Err(err) => {
                                    tracing::error!(error = %err, "udp proxy upstream bind failed");
                                    return;
                                }
                            };
                            if let Err(err) = upstream.connect((container_ip, map.container_port)).await {
                                tracing::error!(%container_ip, container_port = map.container_port, error = %err, "udp proxy upstream connect failed");
                                return;
                            }
                            let mut reply_buf = vec![0u8; 65535];
                            loop {
                                tokio::select! {
                                    incoming = rx.recv() => {
                                        match incoming {
                                            Some((payload, _)) => {
                                                if let Err(err) = upstream.send(&payload).await {
                                                    tracing::warn!(error = %err, "udp proxy forward failed");
                                                }
                                            }
                                            None => return,
                                        }
                                    }
                                    reply = upstream.recv(&mut reply_buf) => {
                                        match reply {
                                            Ok(len) => {
                                                if let Err(err) = host_sock.send_to(&reply_buf[..len], client_addr).await {
                                                    tracing::warn!(error = %err, "udp proxy reply failed");
                                                }
                                            }
                                            Err(_) => return,
                                        }
                                    }
                                }
                            }
                        });
                        tx
                    });
                let _ = sink.send((data, client_addr));
            }
            Err(err) => {
                tracing::warn!(host_port = map.host_port, error = %err, "udp proxy recv failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_map_serde_roundtrip() {
        let map = PortMap::tcp(8080, 3306);
        let value = serde_json::to_value(&map).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "host_port": 8080,
                "container_port": 3306,
                "protocol": "tcp"
            })
        );
        let decoded: PortMap = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, map);

        let udp = PortMap::udp(5353, 53);
        assert_eq!(
            serde_json::from_value::<PortMap>(serde_json::to_value(&udp).unwrap()).unwrap(),
            udp
        );
    }

    #[test]
    fn port_map_validation_rejects_zero_ports() {
        assert!(PortMap::tcp(0, 80).validate().is_err());
        assert!(PortMap::tcp(8080, 0).validate().is_err());
        assert!(PortMap::tcp(8080, 80).validate().is_ok());
    }

    #[test]
    fn duplicate_host_ports_are_rejected() {
        let maps = vec![PortMap::tcp(8080, 3306), PortMap::tcp(8080, 5432)];
        assert!(validate_port_maps(&maps).is_err());
        let maps = vec![PortMap::tcp(8080, 3306), PortMap::udp(8080, 53)];
        assert!(validate_port_maps(&maps).is_err());
    }

    #[test]
    fn retry_delay_grows_and_caps() {
        let first = retry_delay(1);
        assert!(retry_delay(2) > first);
        let capped = retry_delay(9);
        assert_eq!(capped, retry_delay(5));
        assert!(capped <= Duration::from_millis(3200));
    }
}
