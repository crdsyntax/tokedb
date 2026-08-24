//! Execution layer: turns a stored container spec plus its image into a
//! running, isolated database process. Streams logs to the container's state
//! directory and drives the container state machine through start/stop.

use std::fs;
use std::io;
use std::path::Path;

use crate::error::{Result, RuntimeError};
use crate::image::{ImageManifest, ImageStore};
use crate::isolation::{default_allowlist, ContainerUser, SeccompProfile, SecurityProfile};
use crate::network::{PortMap, PortProtocol};
use crate::runtime::process::CommandSpec;
use crate::runtime::{ContainerStore, PortBinding, Protocol};
use crate::state::StateLayout;
use crate::storage::VolumeStore;

#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
use crate::filesystem::{unpack_layer, MountSpec, OverlaySpec, RootfsPrep};
#[cfg(target_os = "linux")]
use crate::image::Image;
#[cfg(target_os = "linux")]
use crate::isolation::{CgroupManager, ResourceLimits as CgroupLimits};
#[cfg(target_os = "linux")]
use crate::network::{
    attach_container,
    bridge::{container_ipv4, ensure_bridge},
    namespace::detach_container,
    port::{spawn_port_proxies, ProxyHandle},
};
#[cfg(target_os = "linux")]
use crate::runtime::{
    process::{spawn_with_prep, ProcessSignal, SpawnedProcess},
    Container, ContainerState,
};

/// The database process runs as this unprivileged user inside the container.
const CONTAINER_UID: u32 = 999;
const CONTAINER_GID: u32 = 999;

#[cfg(target_os = "linux")]
const ROOTFS_DIR: &str = "rootfs";
const LOG_DIR: &str = "logs";
const STDOUT_LOG: &str = "stdout.log";
const STDERR_LOG: &str = "stderr.log";

/// Captured stdout and stderr of a container.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ContainerLogs {
    pub stdout: String,
    pub stderr: String,
}

#[cfg(target_os = "linux")]
const STOP_GRACE_SECS: u64 = 10;
#[cfg(target_os = "linux")]
const KILL_GRACE_SECS: u64 = 5;

/// Read-only host paths made visible inside every container so database
/// binaries and their dynamic dependencies can run. The database program
/// itself comes from the image layers.
#[cfg(target_os = "linux")]
const SYSTEM_BIND_DIRS: &[&str] = &[
    "/bin",
    "/usr/bin",
    "/sbin",
    "/usr/sbin",
    "/usr/lib",
    "/usr/lib64",
    "/lib",
    "/lib64",
];

/// Security profile applied to every container process: no-new-privs, a
/// default capability allowlist, the seccomp denylist, and a drop to an
/// unprivileged database user.
pub fn container_security() -> SecurityProfile {
    SecurityProfile {
        capabilities: default_allowlist(),
        seccomp: Some(SeccompProfile::default_denylist()),
        user: Some(ContainerUser {
            uid: CONTAINER_UID,
            gid: CONTAINER_GID,
        }),
    }
}

/// Builds the runtime command for a container from the image's startup
/// command, wrapped in the container security profile and its own network
/// namespace.
pub fn command_from_image(manifest: &ImageManifest) -> CommandSpec {
    let mut parts = manifest.startup_command.iter();
    let mut spec = CommandSpec::new(parts.next().map(String::as_str).unwrap_or("/bin/sh"));
    for part in parts {
        spec = spec.arg(part.clone());
    }
    spec.security(container_security()).netns(true)
}

/// Parses a `HOST:CONTAINER` port binding; a bare `PORT` binds the same port
/// on both sides.
pub fn parse_port_binding(input: &str) -> Result<PortBinding> {
    let (host, container) = match input.split_once(':') {
        Some((host, container)) => (host, container),
        None => (input, input),
    };
    let host_port: u16 = host.parse().map_err(|_| {
        RuntimeError::InvalidConfig(format!("invalid host port `{host}` in binding `{input}`"))
    })?;
    let container_port: u16 = container.parse().map_err(|_| {
        RuntimeError::InvalidConfig(format!(
            "invalid container port `{container}` in binding `{input}`"
        ))
    })?;
    if host_port == 0 || container_port == 0 {
        return Err(RuntimeError::InvalidConfig(format!(
            "port binding `{input}` must use non-zero ports"
        )));
    }
    Ok(PortBinding {
        host_port,
        container_port,
        protocol: Protocol::Tcp,
    })
}

/// Converts stored port bindings into bridge proxy maps.
pub fn port_maps(ports: &[PortBinding]) -> Vec<PortMap> {
    ports
        .iter()
        .map(|binding| PortMap {
            host_port: binding.host_port,
            container_port: binding.container_port,
            protocol: match binding.protocol {
                Protocol::Tcp => PortProtocol::Tcp,
                Protocol::Udp => PortProtocol::Udp,
            },
        })
        .collect()
}

/// Prints the container's captured stdout and stderr.
pub fn logs(containers: &ContainerStore, layout: &StateLayout, name: &str) -> Result<()> {
    let container = containers.find(name)?;
    let logs_dir = layout.container_dir(&container.id)?.join(LOG_DIR);
    print_log_file(&logs_dir.join(STDOUT_LOG))?;
    print_log_file(&logs_dir.join(STDERR_LOG))
}

fn print_log_file(path: &Path) -> Result<()> {
    match fs::read_to_string(path) {
        Ok(content) => {
            print!("{content}");
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        }),
    }
}

/// Reads the container's captured stdout and stderr as text, missing files
/// treated as empty. Used by the console for live log following.
pub fn read_logs(
    containers: &ContainerStore,
    layout: &StateLayout,
    name: &str,
) -> Result<ContainerLogs> {
    let container = containers.find(name)?;
    let logs_dir = layout.container_dir(&container.id)?.join(LOG_DIR);
    Ok(ContainerLogs {
        stdout: read_log_content(&logs_dir.join(STDOUT_LOG)),
        stderr: read_log_content(&logs_dir.join(STDERR_LOG)),
    })
}

fn read_log_content(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Starts the container in the foreground: builds the rootfs, wires up
/// network and cgroups, spawns the database process, streams its output to
/// the container logs, and blocks until it exits.
pub fn start(
    containers: &ContainerStore,
    images: &ImageStore,
    volumes: &VolumeStore,
    layout: &StateLayout,
    name: &str,
) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (containers, images, volumes, layout, name);
        Err(RuntimeError::UnsupportedPlatform("start"))
    }
    #[cfg(target_os = "linux")]
    {
        start_impl(containers, images, volumes, layout, name)
    }
}

/// Stops a running container: SIGTERM, then SIGKILL after the grace period.
pub fn stop(containers: &ContainerStore, name: &str) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (containers, name);
        Err(RuntimeError::UnsupportedPlatform("stop"))
    }
    #[cfg(target_os = "linux")]
    {
        stop_impl(containers, name)
    }
}

#[cfg(target_os = "linux")]
fn start_impl(
    containers: &ContainerStore,
    images: &ImageStore,
    volumes: &VolumeStore,
    layout: &StateLayout,
    name: &str,
) -> Result<()> {
    let mut container = containers.find(name)?;
    container.state = container.state.transition(ContainerState::Starting)?;
    containers.save(&container)?;

    let image = images.get(&container.image)?;
    let container_dir = layout.container_dir(&container.id)?;

    let mut spec = container.command.clone();
    spec.security = Some(container_security());
    spec.netns = true;
    spec.kill_on_parent_exit = true;
    spec.cwd = Some(PathBuf::from("/"));

    let mut prep = build_rootfs(&image, &container_dir)?;
    prep.bind_mounts.extend(system_bind_mounts());
    for mount in &container.volumes {
        let volume = volumes.get(&mount.name)?;
        prep.bind_mounts
            .push(volume.mount_spec(PathBuf::from(&mount.mount_path), false));
    }

    // Let the unprivileged database user write into the writable layers.
    for dir in [&prep.overlay.upper_dir, &prep.overlay.work_dir] {
        chown_tree(dir, CONTAINER_UID, CONTAINER_GID)?;
    }
    for mount in &prep.bind_mounts {
        if !mount.read_only {
            chown_tree(&mount.source, CONTAINER_UID, CONTAINER_GID)?;
        }
    }

    let logs_dir = container_dir.join(LOG_DIR);
    fs::create_dir_all(&logs_dir).map_err(|err| RuntimeError::Io {
        path: logs_dir.display().to_string(),
        message: err.to_string(),
    })?;

    let mut process = spawn_with_prep(&spec, Some(prep))?;

    let mut network = NetworkSetup::default();
    let mut cgroup = CgroupSetup::default();
    if let Err(err) = wire_up(
        &layout.config().bridge_name,
        &process,
        &container,
        &mut network,
        &mut cgroup,
    ) {
        if pid_alive(process.host_pid()) {
            let _ = process.kill();
            let _ = process.wait();
            cleanup(&mut network, &mut cgroup, &container.id);
            container.state = ContainerState::Stopped;
            container.pid = None;
            let _ = containers.save(&container);
            return Err(err);
        }
        // The container already exited before its network was wired up
        // (e.g. a one-shot startup command). Skip network setup and let the
        // process be reaped normally.
        eprintln!("warning: container `{name}` exited before network setup; skipping");
        cleanup(&mut network, &mut cgroup, &container.id);
    }

    container.state = container.state.transition(ContainerState::Running)?;
    container.pid = Some(process.host_pid());
    containers.save(&container)?;

    println!(
        "container `{name}` started (pid {}, ip {})",
        process.host_pid(),
        container_ipv4(&container.id)
    );

    if let Some(stdout) = process.take_stdout() {
        stream_pipe(stdout, logs_dir.join(STDOUT_LOG), false);
    }
    if let Some(stderr) = process.take_stderr() {
        stream_pipe(stderr, logs_dir.join(STDERR_LOG), true);
    }

    let status = process.wait()?;
    println!("container `{name}` exited with status {status}");

    container.state = ContainerState::Stopped;
    container.pid = None;
    containers.save(&container)?;
    cleanup(&mut network, &mut cgroup, &container.id);
    Ok(())
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct NetworkSetup {
    host_veth: Option<String>,
    proxies: Vec<ProxyHandle>,
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct CgroupSetup {
    enabled: bool,
}

/// Attaches the container to the bridge, publishes its ports, and applies
/// resource limits through cgroups.
#[cfg(target_os = "linux")]
fn wire_up(
    bridge: &str,
    process: &SpawnedProcess,
    container: &Container,
    network: &mut NetworkSetup,
    cgroup: &mut CgroupSetup,
) -> Result<()> {
    ensure_bridge(bridge)?;
    let host_veth = attach_container(bridge, process.host_pid(), &container.id)?;
    network.host_veth = Some(host_veth);

    if !container.ports.is_empty() {
        network.proxies =
            spawn_port_proxies(&port_maps(&container.ports), container_ipv4(&container.id))?;
    }

    if container.resources.memory_bytes.is_some()
        || container.resources.cpu_quota.is_some()
        || container.resources.pids_max.is_some()
    {
        let limits = CgroupLimits {
            cpu_quota: container.resources.cpu_quota,
            memory_bytes: container.resources.memory_bytes,
            pids_max: container.resources.pids_max,
        };
        let manager = CgroupManager::new("/sys/fs/cgroup");
        manager.create(&container.id)?;
        cgroup.enabled = true;
        manager.apply(&container.id, &limits)?;
        manager.attach(&container.id, process.host_pid())?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn cleanup(network: &mut NetworkSetup, cgroup: &mut CgroupSetup, container_id: &str) {
    if let Some(host_veth) = network.host_veth.take() {
        let _ = detach_container(&host_veth);
    }
    network.proxies.clear();
    if cgroup.enabled {
        cgroup.enabled = false;
        let _ = CgroupManager::new("/sys/fs/cgroup").remove(container_id);
    }
}

/// Materializes the image layers into `container_dir/rootfs` and returns the
/// overlay+bind prep used to build the container's view of the filesystem.
#[cfg(target_os = "linux")]
fn build_rootfs(image: &Image, container_dir: &Path) -> Result<RootfsPrep> {
    let rootfs = container_dir.join(ROOTFS_DIR);
    if rootfs.exists() {
        fs::remove_dir_all(&rootfs).map_err(|err| RuntimeError::Io {
            path: rootfs.display().to_string(),
            message: err.to_string(),
        })?;
    }
    fs::create_dir_all(&rootfs).map_err(|err| RuntimeError::Io {
        path: rootfs.display().to_string(),
        message: err.to_string(),
    })?;

    let mut lower_layers = Vec::with_capacity(image.layers.len());
    for (index, layer) in image.layers.iter().enumerate() {
        let lower = rootfs.join(format!("lower{index}"));
        fs::create_dir_all(&lower).map_err(|err| RuntimeError::Io {
            path: lower.display().to_string(),
            message: err.to_string(),
        })?;
        unpack_layer(layer, &lower)?;
        lower_layers.push(lower);
    }

    Ok(RootfsPrep {
        overlay: OverlaySpec {
            lower_layers,
            upper_dir: rootfs.join("upper"),
            work_dir: rootfs.join("work"),
            target: rootfs.join("merged"),
        },
        bind_mounts: Vec::new(),
    })
}

#[cfg(target_os = "linux")]
fn system_bind_mounts() -> Vec<MountSpec> {
    SYSTEM_BIND_DIRS
        .iter()
        .filter(|dir| Path::new(dir).is_dir())
        .map(|dir| MountSpec {
            source: PathBuf::from(dir),
            target: PathBuf::from(dir),
            read_only: true,
        })
        .collect()
}

/// Recursively changes ownership of a directory tree, skipping symlinks.
#[cfg(target_os = "linux")]
fn chown_tree(path: &Path, uid: u32, gid: u32) -> Result<()> {
    use std::os::unix::fs::chown;

    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(RuntimeError::Io {
                path: path.display().to_string(),
                message: err.to_string(),
            })
        }
    };
    for entry in entries {
        let entry = entry.map_err(RuntimeError::from)?;
        let file_type = entry.file_type().map_err(RuntimeError::from)?;
        if file_type.is_symlink() {
            continue;
        }
        let entry_path = entry.path();
        if file_type.is_dir() {
            chown_tree(&entry_path, uid, gid)?;
        }
        chown(&entry_path, Some(uid), Some(gid)).map_err(RuntimeError::from)?;
    }
    chown(path, Some(uid), Some(gid)).map_err(RuntimeError::from)
}

/// Copies a container output pipe to its log file, mirroring it to the CLI
/// (stdout for stdout, stderr for stderr) while the container runs.
#[cfg(target_os = "linux")]
fn stream_pipe(reader: impl Read + Send + 'static, log_path: PathBuf, to_stderr: bool) {
    let file = match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(file) => file,
        Err(err) => {
            eprintln!("warning: cannot open log {}: {err}", log_path.display());
            return;
        }
    };
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut file = file;
        let mut buffer = [0u8; 8192];
        loop {
            let n = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            let _ = file.write_all(&buffer[..n]);
            let _ = file.flush();
            if to_stderr {
                let _ = io::stderr().write_all(&buffer[..n]);
            } else {
                let _ = io::stdout().write_all(&buffer[..n]);
            }
        }
    });
}

#[cfg(target_os = "linux")]
fn stop_impl(containers: &ContainerStore, name: &str) -> Result<()> {
    let mut container = containers.find(name)?;
    if container.state != ContainerState::Running {
        return Err(RuntimeError::ContainerNotRunning { id: container.id });
    }
    let pid = container.pid.ok_or_else(|| RuntimeError::CorruptState {
        id: container.id.clone(),
        reason: "running container without a recorded pid".into(),
    })?;

    container.state = container.state.transition(ContainerState::Stopping)?;
    containers.save(&container)?;

    signal(pid, ProcessSignal::Term)?;
    if !wait_for_exit(pid, STOP_GRACE_SECS) {
        signal(pid, ProcessSignal::Kill)?;
        wait_for_exit(pid, KILL_GRACE_SECS);
    }

    container.state = ContainerState::Stopped;
    container.pid = None;
    containers.save(&container)?;
    println!("container `{name}` stopped");
    Ok(())
}

#[cfg(target_os = "linux")]
fn signal(pid: u32, process_signal: ProcessSignal) -> Result<()> {
    use nix::sys::signal::{kill, Signal};

    let native = match process_signal {
        ProcessSignal::Term => Signal::SIGTERM,
        ProcessSignal::Kill => Signal::SIGKILL,
    };
    let target = nix::unistd::Pid::from_raw(pid as i32);
    match kill(target, native) {
        Ok(()) => Ok(()),
        Err(nix::errno::Errno::ESRCH) => Ok(()),
        Err(err) => Err(RuntimeError::from(err)),
    }
}

#[cfg(target_os = "linux")]
fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(target_os = "linux")]
fn wait_for_exit(pid: u32, grace_secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(grace_secs);
    while std::time::Instant::now() < deadline {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn parse_port_binding_accepts_bare_and_mapped_forms() {
        let bare = parse_port_binding("3306").unwrap();
        assert_eq!(bare.host_port, 3306);
        assert_eq!(bare.container_port, 3306);
        assert_eq!(bare.protocol, Protocol::Tcp);

        let mapped = parse_port_binding("18080:3306").unwrap();
        assert_eq!(mapped.host_port, 18080);
        assert_eq!(mapped.container_port, 3306);
    }

    #[test]
    fn parse_port_binding_rejects_garbage_and_zero() {
        for bad in ["nope", "0:3306", "1:99999", "1:2:3", ""] {
            assert!(
                matches!(parse_port_binding(bad), Err(RuntimeError::InvalidConfig(_))),
                "expected rejection for `{bad}`"
            );
        }
    }

    #[test]
    fn port_maps_converts_bindings() {
        let maps = port_maps(&[PortBinding {
            host_port: 80,
            container_port: 8080,
            protocol: Protocol::Tcp,
        }]);
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].host_port, 80);
        assert_eq!(maps[0].container_port, 8080);
        assert_eq!(maps[0].protocol, PortProtocol::Tcp);
    }

    #[test]
    fn command_from_image_uses_startup_command_and_security() {
        let manifest = crate::image::manifest::sample_manifest();
        let spec = command_from_image(&manifest);
        assert_eq!(spec.program, PathBuf::from("mariadbd"));
        assert!(spec.args.is_empty());
        assert!(spec.netns);
        let profile = spec.security.unwrap();
        assert_eq!(profile.user.unwrap().uid, CONTAINER_UID);
        assert!(profile.seccomp.is_some());
        assert_eq!(profile.capabilities, default_allowlist());
    }
}
