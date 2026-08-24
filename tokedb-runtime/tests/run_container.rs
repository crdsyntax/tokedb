#![cfg(all(target_os = "linux", feature = "integration-linux"))]

//! End-to-end container execution tests: rootfs build, volume bind mounts,
//! log capture, port publishing and graceful stop. Require root (WSL2).

use std::fs;
use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use tokedb_runtime::config::RuntimeConfig;
use tokedb_runtime::image::manifest::{Healthcheck, ImageManifest, LayerRef};
use tokedb_runtime::image::{Architecture, ImageStore};
use tokedb_runtime::runtime::container::ContainerSpec;
use tokedb_runtime::runtime::lifecycle::ContainerState;
use tokedb_runtime::runtime::{run, ContainerStore, ResourceLimits, VolumeMount};
use tokedb_runtime::state::StateLayout;
use tokedb_runtime::storage::VolumeStore;

/// All containers share the 10.20.0.0/24 subnet, so tests touching the
/// network stack must not run concurrently with each other.
static RUN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn guard() -> std::sync::MutexGuard<'static, ()> {
    RUN_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Derives a stable short bridge name from a unique temp path. Each test
/// gets its own bridge so the tests never interfere with each other (or
/// with tests/network.rs) on the shared subnet, where only the last bridge
/// created on the subnet owns the route.
fn bridge_name_for(path: &Path) -> String {
    let stem = path.file_name().unwrap().to_string_lossy();
    format!("db{}", &stem[stem.len().saturating_sub(6)..])
}

struct TestEnv {
    _temp: tempfile::TempDir,
    config: RuntimeConfig,
}

impl TestEnv {
    fn new(script: &str) -> TestEnv {
        if !nix::unistd::Uid::effective().is_root() {
            panic!("these tests require root (WSL2)");
        }
        let temp = tempfile::tempdir().unwrap();
        let mut config = RuntimeConfig::new(temp.path().to_path_buf());
        // Each test gets its own bridge so the tests never interfere with
        // each other (or with tests/network.rs) on the shared 10.20.0.0/24
        // subnet, where only the last-created bridge owns the route.
        config.bridge_name = bridge_name_for(temp.path());
        let layout = StateLayout::new(config.clone());
        layout.ensure_directories().unwrap();
        let env = TestEnv {
            _temp: temp,
            config,
        };
        let bundle = build_bundle(env.config.data_root.join("bundle.tar.gz"), script);
        env.images().import_bundle(&bundle).unwrap();
        env
    }

    fn images(&self) -> ImageStore {
        ImageStore::new(self.config.images_dir.clone())
    }

    fn containers(&self) -> ContainerStore {
        ContainerStore::new(StateLayout::new(self.config.clone()))
    }

    fn volumes(&self) -> VolumeStore {
        VolumeStore::new(self.config.volumes_dir.clone())
    }

    fn layout(&self) -> StateLayout {
        StateLayout::new(self.config.clone())
    }

    fn volume_path(&self, name: &str) -> PathBuf {
        self.config.volumes_dir.join(name)
    }

    fn stdout_log(&self, name: &str) -> String {
        let container = self.containers().find(name).unwrap();
        let path = self
            .layout()
            .container_dir(&container.id)
            .unwrap()
            .join("logs")
            .join("stdout.log");
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
    }

    fn create(&self, name: &str, ports: &[&str]) {
        let image = self.images().get("testdb:1.0").unwrap();
        let mut bindings = Vec::new();
        for raw in ports {
            bindings.push(run::parse_port_binding(raw).unwrap());
        }
        let container = self.containers().create(ContainerSpec {
            name: name.into(),
            image: image.reference.clone(),
            command: run::command_from_image(&image.manifest),
            resources: ResourceLimits::default(),
            volumes: vec![VolumeMount {
                name: format!("{name}-data"),
                mount_path: image.manifest.data_directory.clone(),
            }],
            ports: bindings,
        });
        container.unwrap();
        self.volumes().create(&format!("{name}-data")).unwrap();
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = tokedb_runtime::network::delete_bridge(&self.config.bridge_name);
    }
}

/// Builds an importable bundle whose startup command runs the given script
/// through `/bin/bash` (provided by the host's read-only bind mounts). The
/// single layer carries a `db.txt` file so rootfs visibility is verified.
fn build_bundle(dest: PathBuf, script: &str) -> PathBuf {
    let layer_bytes = {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "db.txt", "hello".as_bytes())
                .unwrap();
            builder.finish().unwrap();
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    };
    let hex = format!("{:x}", Sha256::digest(&layer_bytes));

    let mut manifest = ImageManifest {
        database: "testdb".into(),
        version: "1.0".into(),
        architecture: Architecture::Amd64,
        digest: String::new(),
        default_port: 3306,
        data_directory: "/var/lib/mysql".into(),
        healthcheck: Healthcheck {
            port: 3306,
            timeout_secs: 5,
        },
        startup_command: vec!["/bin/bash".into(), "-c".into(), script.into()],
        layers: vec![LayerRef {
            digest: format!("sha256:{hex}"),
            size: layer_bytes.len() as u64,
        }],
    };
    manifest.digest = manifest.compute_digest().unwrap();

    let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    let bundle = dest.to_path_buf();
    let file = fs::File::create(&bundle).unwrap();
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut manifest_header = tar::Header::new_gnu();
    manifest_header.set_size(manifest_bytes.len() as u64);
    manifest_header.set_mode(0o644);
    manifest_header.set_cksum();
    builder
        .append_data(&mut manifest_header, "manifest.json", &manifest_bytes[..])
        .unwrap();
    let mut layer_header = tar::Header::new_gnu();
    layer_header.set_size(layer_bytes.len() as u64);
    layer_header.set_mode(0o644);
    layer_header.set_cksum();
    builder
        .append_data(
            &mut layer_header,
            format!("layers/{hex}.tar.gz"),
            &layer_bytes[..],
        )
        .unwrap();
    let encoder = builder.into_inner().unwrap();
    encoder.finish().unwrap();
    bundle
}

fn wait_for_state(containers: &ContainerStore, name: &str, state: ContainerState, secs: u64) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if containers.find(name).unwrap().state == state {
            return;
        }
        if Instant::now() > deadline {
            panic!("container `{name}` never reached {state:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn start_runs_command_and_persists_logs_and_volume() {
    let _guard = guard();
    let env = TestEnv::new(
        "echo hello-from-container; cat /db.txt; \
         mkdir -p /var/lib/mysql; echo persisted > /var/lib/mysql/marker.txt",
    );
    env.create("t1", &[]);

    run::start(
        &env.containers(),
        &env.images(),
        &env.volumes(),
        &env.layout(),
        "t1",
    )
    .unwrap();

    let container = env.containers().find("t1").unwrap();
    assert_eq!(container.state, ContainerState::Stopped);
    assert!(container.pid.is_none());

    assert_eq!(
        fs::read_to_string(env.volume_path("t1-data").join("marker.txt")).unwrap(),
        "persisted\n"
    );
    let stdout = env.stdout_log("t1");
    assert!(
        stdout.contains("hello-from-container"),
        "stdout log must contain the container echo, got: {stdout}"
    );
    assert!(
        stdout.contains("hello"),
        "lower layer must be visible in the container rootfs, got: {stdout}"
    );
}

#[test]
fn stop_terminates_running_container() {
    let _guard = guard();
    let env = TestEnv::new("exec sleep 300");
    env.create("t2", &[]);

    let thread_config = env.config.clone();
    let handle = std::thread::spawn(move || {
        let config = thread_config;
        let layout = StateLayout::new(config.clone());
        run::start(
            &ContainerStore::new(layout.clone()),
            &ImageStore::new(config.images_dir.clone()),
            &VolumeStore::new(config.volumes_dir.clone()),
            &layout,
            "t2",
        )
        .unwrap();
    });

    wait_for_state(&env.containers(), "t2", ContainerState::Running, 30);
    run::stop(&env.containers(), "t2").unwrap();
    handle.join().unwrap();

    let container = env.containers().find("t2").unwrap();
    assert_eq!(container.state, ContainerState::Stopped);
    assert!(container.pid.is_none());
}

#[test]
fn published_port_is_reachable_while_running() {
    let _guard = guard();
    let env = TestEnv::new("exec python3 -m http.server 8000 --directory /var/lib/mysql");
    env.create("t3", &["18080:8000"]);

    let thread_config = env.config.clone();
    let handle = std::thread::spawn(move || {
        let config = thread_config;
        let layout = StateLayout::new(config.clone());
        run::start(
            &ContainerStore::new(layout.clone()),
            &ImageStore::new(config.images_dir.clone()),
            &VolumeStore::new(config.volumes_dir.clone()),
            &layout,
            "t3",
        )
        .unwrap();
    });

    wait_for_state(&env.containers(), "t3", ContainerState::Running, 30);

    let deadline = Instant::now() + Duration::from_secs(40);
    let mut reachable = false;
    while Instant::now() < deadline {
        match TcpStream::connect_timeout(
            &"127.0.0.1:18080".parse().unwrap(),
            Duration::from_secs(3),
        ) {
            Ok(_) => {
                reachable = true;
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(300)),
        }
    }
    assert!(
        reachable,
        "published port 18080 never became reachable through the proxy"
    );

    run::stop(&env.containers(), "t3").unwrap();
    handle.join().unwrap();

    let container = env.containers().find("t3").unwrap();
    assert_eq!(container.state, ContainerState::Stopped);
}
