



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
use crate::storage::{LayerStore, VolumeStore};

#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::Command;

#[cfg(target_os = "linux")]
use crate::filesystem::{MountSpec, OverlaySpec, RootfsPrep};
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
    container::DbUser,
    process::{spawn_with_prep, ProcessSignal, SpawnedProcess},
    Container, ContainerState,
};


const CONTAINER_UID: u32 = 999;
const CONTAINER_GID: u32 = 999;

#[cfg(target_os = "linux")]
const ROOTFS_DIR: &str = "rootfs";
const LOG_DIR: &str = "logs";
const STDOUT_LOG: &str = "stdout.log";
const STDERR_LOG: &str = "stderr.log";


#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ContainerLogs {
    pub stdout: String,
    pub stderr: String,
}

#[cfg(target_os = "linux")]
const STOP_GRACE_SECS: u64 = 10;
#[cfg(target_os = "linux")]
const KILL_GRACE_SECS: u64 = 5;




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




pub fn command_from_image(manifest: &ImageManifest) -> CommandSpec {
    let mut parts = manifest.startup_command.iter();
    let mut spec = CommandSpec::new(parts.next().map(String::as_str).unwrap_or("/bin/sh"));
    for part in parts {
        spec = spec.arg(part.clone());
    }
    spec.security(container_security()).netns(true)
}



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




pub fn start(
    containers: &ContainerStore,
    images: &ImageStore,
    volumes: &VolumeStore,
    layers: &LayerStore,
    layout: &StateLayout,
    name: &str,
) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (containers, images, volumes, layers, layout, name);
        Err(RuntimeError::UnsupportedPlatform("start"))
    }
    #[cfg(target_os = "linux")]
    {
        start_impl(containers, images, volumes, layers, layout, name)
    }
}


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
fn init_user_sql(engine: &str, user: &DbUser) -> Option<String> {
    if user.username.trim().is_empty() || user.password.is_empty() {
        return None;
    }
    match engine {
        "mariadb" | "mysql" => {
            let u = sql_literal(&user.username);
            let p = sql_literal(&user.password);
            Some(format!(
                "CREATE USER IF NOT EXISTS '{u}'@'%' IDENTIFIED BY '{p}';\n\
                 ALTER USER '{u}'@'%' IDENTIFIED BY '{p}';\n\
                 GRANT ALL PRIVILEGES ON *.* TO '{u}'@'%' WITH GRANT OPTION;\n\
                 FLUSH PRIVILEGES;\n"
            ))
        }
        _ => None,
    }
}


#[cfg(target_os = "linux")]
fn sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}






#[cfg(target_os = "linux")]
fn engine_init_spec(
    engine: &str,
) -> Option<(&'static str, Vec<String>, &'static str)> {
    match engine {
        "mariadb" => Some((
            "mariadb-install-db",
            vec![
                "--user=root".to_string(),
                "--auth-root-authentication-method=normal".to_string(),
            ],
            "mysql",
        )),
        "mysql" => Some((
            "mysql_install_db",
            vec!["--user=root".to_string()],
            "mysql",
        )),
        _ => None,
    }
}





#[cfg(target_os = "linux")]
fn maybe_init_data_directory(engine: &str, host_path: &Path) -> Result<()> {
    let Some((bin, mut args, marker)) = engine_init_spec(engine) else {
        return Ok(());
    };
    if host_path.join(marker).exists() {
        return Ok(());
    }
    
    
    
    if dir_has_content(host_path)? {
        eprintln!("data directory at {path} not initialized; clearing partial contents", path = host_path.display());
        clear_dir(host_path)?;
    }
    let datadir = host_path
        .to_str()
        .ok_or_else(|| RuntimeError::InvalidConfig(format!("ruta inválida: {host_path:?}")))?;
    
    
    
    
    args.push(format!("--datadir={datadir}"));
    eprintln!("initializing {engine} data directory at {datadir}");
    let status = Command::new(bin)
        .args(&args)
        .status()
        .map_err(|err| RuntimeError::Process(format!("no se pudo ejecutar `{bin}`: {err}")))?;
    if !status.success() {
        return Err(RuntimeError::Process(format!(
            "`{bin}` falló al inicializar el data directory (estado {status})"
        )));
    }
    Ok(())
}



#[cfg(target_os = "linux")]
fn dir_has_content(path: &Path) -> Result<bool> {
    let mut entries = fs::read_dir(path).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(|err| RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        if entry.file_name() != "lost+found" {
            return Ok(true);
        }
    }
    Ok(false)
}



#[cfg(target_os = "linux")]
fn clear_dir(path: &Path) -> Result<()> {
    let mut entries = fs::read_dir(path).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(|err| RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        let target = entry.path();
        
        if entry.file_name() == std::ffi::OsStr::new(".tokedb-volume") {
            continue;
        }
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            fs::remove_dir_all(&target)
        } else {
            fs::remove_file(&target)
        }
        .map_err(|err| RuntimeError::Io {
            path: target.display().to_string(),
            message: err.to_string(),
        })?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn start_impl(
    containers: &ContainerStore,
    images: &ImageStore,
    volumes: &VolumeStore,
    layers: &LayerStore,
    layout: &StateLayout,
    name: &str,
) -> Result<()> {
    let mut container = containers.find(name)?;
    container.state = container.state.transition(ContainerState::Starting)?;
    containers.save(&container)?;

    let image = images.get(&container.image)?;
    let container_dir = layout.container_dir(&container.id)?;

    
    
    
    
    let mut lower_layers: Vec<PathBuf> = Vec::with_capacity(image.manifest.layers.len());
    let mut newly_acquired: Vec<String> = Vec::new();
    for (layer_ref, tar_gz) in image.manifest.layers.iter().zip(image.layers.iter()) {
        let digest = layer_ref.digest.clone();
        if !container.acquired_layers.contains(&digest) {
            layers.ensure(&digest, tar_gz)?;
            newly_acquired.push(digest.clone());
        }
        lower_layers.push(layers.diff_path(&digest)?);
    }
    if !newly_acquired.is_empty() {
        container.acquired_layers.extend(newly_acquired);
        containers.save(&container)?;
    }

    let mut spec = container.command.clone();
    spec.security = Some(container_security());
    spec.netns = true;
    spec.kill_on_parent_exit = true;
    spec.cwd = Some(PathBuf::from("/"));

    let mut prep = build_rootfs(&lower_layers, &container_dir)?;
    prep.bind_mounts.extend(system_bind_mounts());
    for mount in &container.volumes {
        let volume = volumes.get(&mount.name)?;
        prep.bind_mounts
            .push(volume.mount_spec(PathBuf::from(&mount.mount_path), false));
    }

    
    
    
    
    let mut init_file_arg: Option<String> = None;
    for mount in &container.volumes {
        if mount.mount_path == image.manifest.data_directory {
            let volume = volumes.get(&mount.name)?;
            maybe_init_data_directory(&image.manifest.database, &volume.path)?;
            
            
            
            
            if let Some(sql) = init_user_sql(&image.manifest.database, &container.db_user) {
                let host_file = volume.path.join("tokedb-init.sql");
                fs::write(&host_file, sql).map_err(|err| RuntimeError::Io {
                    path: host_file.display().to_string(),
                    message: err.to_string(),
                })?;
                init_file_arg =
                    Some(format!("--init-file={}/tokedb-init.sql", mount.mount_path));
            }
        }
    }
    if let Some(arg) = init_file_arg {
        spec.args.push(arg);
    }

    
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





#[cfg(target_os = "linux")]
fn build_rootfs(lower_layers: &[PathBuf], container_dir: &Path) -> Result<RootfsPrep> {
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

    Ok(RootfsPrep {
        overlay: OverlaySpec {
            lower_layers: lower_layers.to_vec(),
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
