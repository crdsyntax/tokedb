#![cfg(all(target_os = "linux", feature = "integration-linux"))]

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream, UdpSocket as StdUdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nix::mount::umount;
use tokedb_runtime::filesystem::{
    mounts::MountSpec, overlay::OverlaySpec, prepare_container_root, RootfsPrep,
};
use tokedb_runtime::network::{self, attach_container, ensure_bridge, PortMap};
use tokedb_runtime::runtime::process::{spawn_with_prep, CommandSpec, SpawnedProcess};

const MARKER: &str = "marker-network-probe.txt";

/// Every test creates its own bridge on the shared 10.20.0.0/24 subnet, and
/// only the last bridge owns the host route for that prefix. The tests must
/// therefore run serially.
static NETTEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn nettest_guard() -> std::sync::MutexGuard<'static, ()> {
    NETTEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_unique(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{tag}{nanos}")
}

struct TestNet {
    bridge: String,
    work: tempfile::TempDir,
    merged: PathBuf,
    veths: Vec<String>,
}

impl TestNet {
    fn new() -> TestNet {
        if !nix::unistd::Uid::effective().is_root() {
            panic!("these tests require root (WSL2)");
        }
        let overlay_supported = fs::read_to_string("/proc/filesystems")
            .map(|contents| contents.lines().any(|line| line.contains("overlay")))
            .unwrap_or(false);
        if !overlay_supported {
            panic!("overlayfs not available");
        }
        if !Path::new("/usr/bin/python3").exists() {
            panic!("python3 not available");
        }

        let work = tempfile::tempdir().unwrap();
        let root = work.path();
        let lower = root.join("lower");
        let upper = root.join("upper");
        let workdir = root.join("work");
        let merged = root.join("merged");
        for dir in [&lower, &upper, &workdir, &merged] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(lower.join(MARKER), "found-me").unwrap();

        let bridge = format!(
            "db{}",
            now_unique("").chars().rev().take(9).collect::<String>()
        );
        // The runtime's persistent bridge (`db0`) occupies the same subnet
        // after any `tokedb start`. Two bridges on 10.20.0.0/24 leave both
        // kernel routes installed and traffic can go to the idle one, so the
        // test bridge must own the subnet exclusively.
        let _ = tokedb_runtime::network::bridge::delete_bridge("db0");
        ensure_bridge(&bridge).unwrap();
        TestNet {
            bridge,
            work,
            merged,
            veths: Vec::new(),
        }
    }

    fn prep(&self) -> RootfsPrep {
        let lower = self.work.path().join("lower");
        let upper = self.work.path().join("upper");
        let workdir = self.work.path().join("work");
        let merged = self.work.path().join("merged");
        let mut binds = system_bind_mounts();
        for host in ["/etc"] {
            if Path::new(host).exists() {
                binds.push(MountSpec {
                    source: PathBuf::from(host),
                    target: PathBuf::from(host),
                    read_only: true,
                });
            }
        }
        RootfsPrep {
            overlay: OverlaySpec {
                lower_layers: vec![lower],
                upper_dir: upper,
                work_dir: workdir,
                target: merged.clone(),
            },
            bind_mounts: binds,
        }
    }

    fn attach(&mut self, process: &SpawnedProcess, id: &str) -> Ipv4Addr {
        let hosted = attach_container(&self.bridge, process.host_pid(), id).unwrap();
        self.veths.push(hosted);
        network::container_ipv4(id)
    }
}

impl Drop for TestNet {
    fn drop(&mut self) {
        for veth in &self.veths {
            let _ = tokedb_runtime::network::namespace::detach_container(veth);
        }
        let _ = tokedb_runtime::network::delete_bridge(&self.bridge);
        let _ = umount(&self.merged);
    }
}

fn system_bind_mounts() -> Vec<MountSpec> {
    ["/bin", "/usr/bin", "/usr/lib", "/lib", "/lib64"]
        .iter()
        .filter(|host| Path::new(*host).exists())
        .map(|host| MountSpec {
            source: PathBuf::from(host),
            target: PathBuf::from(host),
            read_only: true,
        })
        .collect()
}

fn spawn_netns(prep: &RootfsPrep, script: &str) -> SpawnedProcess {
    let spec = CommandSpec::new("/bin/bash")
        .arg("-c")
        .arg(script)
        .netns(true)
        .cwd("/")
        .kill_on_parent_exit(false);
    spawn_with_prep(&spec, Some(prep.clone())).unwrap()
}

fn read_all_stdout(process: &mut SpawnedProcess) -> String {
    let mut stdout = String::new();
    process
        .take_stdout()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    stdout
}

fn terminate(mut process: SpawnedProcess) {
    let _ = process.kill();
    let _ = process.wait();
}

#[test]
fn two_containers_see_each_other_over_bridge() {
    let _guard = nettest_guard();
    let mut net = TestNet::new();
    let prep = net.prep();

    let server_id = now_unique("srv");
    let server = spawn_netns(&prep, "exec python3 -m http.server 8000");
    let server_ip = net.attach(&server, &server_id);

    let client_script = format!(
        "for i in $(seq 1 30); do \
           if (exec 3<>/dev/tcp/{ip}/8000) 2>/dev/null; then \
             exec 3<>/dev/tcp/{ip}/8000; \
             printf 'GET / HTTP/1.0\\r\\nConnection: close\\r\\n\\r\\n' >&3; \
             cat <&3; exit 0; \
           fi; sleep 1; \
         done; exit 1",
        ip = server_ip
    );
    let client_id = now_unique("cli");
    let mut client = spawn_netns(&prep, &client_script);
    net.attach(&client, &client_id);

    let out = read_all_stdout(&mut client);
    if !out.contains(MARKER) {
        let mut server = server;
        let _ = server.kill();
        let srv_out = read_all_stdout(&mut server);
        panic!("client got: {out}\nserver got: {srv_out}");
    }
    assert!(client.wait().unwrap().success());

    terminate(server);
}

#[test]
fn host_reaches_container_via_userland_port_proxy() {
    let _guard = nettest_guard();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let mut net = TestNet::new();
    let prep = net.prep();

    let script = "python3 -m http.server 8000 2>/dev/null & \
                  exec python3 -c 'import socket; \
                  s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); \
                  s.bind((\"0.0.0.0\",8001)); \
                  [s.sendto(b\"PONG \"+d, a) for d,a in iter(lambda: s.recvfrom(1024), None)]'";
    let server_id = now_unique("srv");
    let mut server = spawn_netns(&prep, script);
    let server_ip = net.attach(&server, &server_id);

    let maps = vec![PortMap::tcp(18080, 8000), PortMap::udp(18081, 8001)];
    let proxies = network::port::spawn_port_proxies(&maps, server_ip).unwrap();

    let mut tcp =
        TcpStream::connect_timeout(&"127.0.0.1:18080".parse().unwrap(), Duration::from_secs(20))
            .expect("proxy tcp reachable");
    tcp.write_all(b"GET / HTTP/1.0\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut page = String::new();
    tcp.read_to_string(&mut page).unwrap();
    assert!(
        page.contains(MARKER),
        "proxied tcp page must expose the marker; got: {page}"
    );

    let udp = StdUdpSocket::bind("0.0.0.0:0").unwrap();
    udp.connect("127.0.0.1:18081").unwrap();
    udp.send_to(b"ping-one", "127.0.0.1:18081").unwrap();
    udp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut reply = [0u8; 128];
    let (len, _) = udp.recv_from(&mut reply).unwrap();
    assert_eq!(&reply[..len], b"PONG ping-one");
    udp.send_to(b"ping-two", "127.0.0.1:18081").unwrap();
    let (len, _) = udp.recv_from(&mut reply).unwrap();
    assert_eq!(&reply[..len], b"PONG ping-two");

    drop(proxies);
    terminate(server);
}

#[test]
fn container_reaches_host_gateway_over_bridge() {
    let _guard = nettest_guard();
    let mut net = TestNet::new();
    let prep = net.prep();

    let listener = TcpListener::bind((Ipv4Addr::new(10, 20, 0, 1), 19000)).unwrap();
    listener.set_nonblocking(true).unwrap();

    let script = "for i in $(seq 1 30); do \
                  (exec 3<>/dev/tcp/10.20.0.1/19000) 2>/dev/null && break; \
                  sleep 1; \
                done; \
                exec 3<>/dev/tcp/10.20.0.1/19000 || exit 1; \
                printf 'hello-gw' >&3; \
                IFS= read -r line <&3; \
                echo GOT:$line";
    let client_id = now_unique("cli");
    let mut client = spawn_netns(&prep, script);
    net.attach(&client, &client_id);

    // The client probes the port once per retry; each probe opens a real
    // connection that dies immediately. Accept until one actually carries
    // the greeting.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
    let mut socket = None;
    while std::time::Instant::now() < deadline {
        match listener.accept() {
            Ok((mut connection, _)) => {
                connection
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut hello = [0u8; 8];
                if connection.read_exact(&mut hello).is_ok() && &hello == b"hello-gw" {
                    socket = Some(connection);
                    break;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(err) => panic!("gateway listener: {err}"),
        }
    }
    let Some(mut socket) = socket else {
        panic!("gateway: container never delivered hello-gw within 40s");
    };
    socket.write_all(b"PONG hello-gw\n").unwrap();

    let out = read_all_stdout(&mut client);
    assert!(
        out.contains("GOT:PONG hello-gw"),
        "container must reach the host gateway; got: {out}"
    );
    assert!(client.wait().unwrap().success());
}
