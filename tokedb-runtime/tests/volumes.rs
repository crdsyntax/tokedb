#![cfg(all(target_os = "linux", feature = "integration-linux"))]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use nix::mount::umount;
use tokedb_runtime::config::RuntimeConfig;
use tokedb_runtime::filesystem::{mounts::MountSpec, overlay::OverlaySpec, RootfsPrep};
use tokedb_runtime::runtime::process::{spawn_with_prep, CommandSpec};
use tokedb_runtime::state::StateLayout;
use tokedb_runtime::storage::{backup_volume, VolumeStore};

#[test]
fn container_destroy_does_not_delete_volume() {
    let work = tempfile::tempdir().unwrap();
    let config = RuntimeConfig::new(work.path().to_path_buf());
    let layout = StateLayout::new(config.clone());
    layout.ensure_directories().unwrap();

    let store = VolumeStore::new(config.volumes_dir.clone());
    let volume = store.create("data").unwrap();
    fs::write(volume.path.join("rows.bin"), b"persistent").unwrap();

    let container_dir = layout.container_dir("c1").unwrap();
    fs::create_dir_all(&container_dir).unwrap();
    fs::write(
        layout.metadata_path("c1").unwrap(),
        r#"{"id":"c1","state":"running"}"#,
    )
    .unwrap();

    fs::remove_dir_all(&container_dir).unwrap();

    assert!(!container_dir.exists());
    assert!(volume.path.is_dir());
    assert_eq!(
        fs::read(volume.path.join("rows.bin")).unwrap(),
        b"persistent"
    );
    assert!(store.get("data").unwrap().path.is_dir());
}

#[test]
fn volume_binds_into_container_data_directory_and_persists() {
    if !nix::unistd::Uid::effective().is_root() {
        eprintln!("skipping: requires root");
        return;
    }
    let overlay_supported = fs::read_to_string("/proc/filesystems")
        .map(|contents| contents.lines().any(|line| line.contains("overlay")))
        .unwrap_or(false);
    if !overlay_supported {
        eprintln!("skipping: overlayfs not available");
        return;
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
    fs::create_dir_all(lower.join("var/lib/mysql")).unwrap();
    fs::create_dir_all(lower.join("var/lib").join("mysql-lower-only")).unwrap();
    for sub in ["bin", "lib", "lib64", "usr/lib"] {
        fs::create_dir_all(lower.join(sub)).unwrap();
    }

    let store = VolumeStore::new(root.join("volumes"));
    let volume = store.create("data").unwrap();
    fs::write(volume.path.join("seed.txt"), b"seeded").unwrap();

    let prep = RootfsPrep {
        overlay: OverlaySpec {
            lower_layers: vec![lower],
            upper_dir: upper.clone(),
            work_dir: workdir,
            target: merged.clone(),
        },
        bind_mounts: vec![volume.mount_spec(PathBuf::from("/var/lib/mysql"), false)]
            .into_iter()
            .chain(system_bind_mounts())
            .collect(),
    };

    let spec = CommandSpec::new("/bin/sh")
        .arg("-c")
        .arg(
            "cat /var/lib/mysql/seed.txt && \
             echo from-container > /var/lib/mysql/written.txt && \
             echo WROTE",
        )
        .kill_on_parent_exit(false);

    let mut spawned = spawn_with_prep(&spec, Some(prep)).unwrap();
    let mut stdout = String::new();
    spawned
        .take_stdout()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert_eq!(stdout.trim(), "seededWROTE");
    assert!(spawned.wait().unwrap().success());

    drop(spawned);
    let _ = umount(&merged);

    assert_eq!(
        fs::read(volume.path.join("written.txt")).unwrap(),
        b"from-container\n"
    );
    assert_eq!(fs::read(volume.path.join("seed.txt")).unwrap(), b"seeded");
}

#[test]
fn backup_snapshot_contains_volume_data() {
    let work = tempfile::tempdir().unwrap();
    let store = VolumeStore::new(work.path().join("volumes"));
    let volume = store.create("data").unwrap();
    fs::create_dir_all(volume.path.join("db")).unwrap();
    fs::write(volume.path.join("db").join("rows.bin"), b"12345").unwrap();

    let archive = backup_volume(&volume, &work.path().join("backups")).unwrap();

    let mut found = false;
    let archive_file = std::fs::File::open(&archive).unwrap();
    let mut reader = tar::Archive::new(archive_file);
    for entry in reader.entries().unwrap() {
        let path = entry
            .unwrap()
            .path()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if path.ends_with("db/rows.bin") || path.ends_with("db\\rows.bin") {
            found = true;
        }
    }
    assert!(found, "archive must contain the volume data file");
}

fn system_bind_mounts() -> Vec<MountSpec> {
    ["/bin", "/lib", "/lib64", "/usr/lib"]
        .iter()
        .filter(|host| Path::new(*host).exists())
        .map(|host| MountSpec {
            source: PathBuf::from(host),
            target: PathBuf::from(host),
            read_only: true,
        })
        .collect()
}
