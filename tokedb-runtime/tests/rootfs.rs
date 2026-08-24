#![cfg(all(target_os = "linux", feature = "integration-linux"))]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use nix::mount::umount;
use tokedb_runtime::filesystem::{
    mounts::MountSpec, overlay::OverlaySpec, prepare_container_root, RootfsPrep,
};
use tokedb_runtime::runtime::process::{spawn_with_prep, CommandSpec};

#[test]
fn container_runs_with_overlay_rootfs_and_pivot() {
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
    let lower = work.path().join("lower");
    let upper = work.path().join("upper");
    let workdir = work.path().join("work");
    let merged = work.path().join("merged");
    for dir in [&lower, &upper, &workdir, &merged] {
        fs::create_dir_all(dir).unwrap();
    }

    for sub in ["bin", "lib", "lib64", "usr/lib"] {
        fs::create_dir_all(lower.join(sub)).unwrap();
    }
    fs::write(lower.join("marker.txt"), "ok").unwrap();

    let prep = RootfsPrep {
        overlay: OverlaySpec {
            lower_layers: vec![lower],
            upper_dir: upper,
            work_dir: workdir,
            target: merged.clone(),
        },
        bind_mounts: system_bind_mounts(),
    };

    let spec = CommandSpec::new("/bin/sh")
        .arg("-c")
        .arg("test -f /marker.txt && echo ROOTED")
        .kill_on_parent_exit(false);

    let mut spawned = spawn_with_prep(&spec, Some(prep)).unwrap();
    let mut stdout = String::new();
    spawned
        .take_stdout()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert_eq!(stdout.trim(), "ROOTED");
    assert!(spawned.wait().unwrap().success());

    drop(spawned);
    let _ = umount(&merged);
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

#[test]
fn prepare_container_root_rejects_empty_overlay() {
    let prep = RootfsPrep {
        overlay: OverlaySpec {
            lower_layers: Vec::new(),
            upper_dir: PathBuf::from("/tmp/upper"),
            work_dir: PathBuf::from("/tmp/work"),
            target: PathBuf::from("/tmp/merged"),
        },
        bind_mounts: Vec::new(),
    };
    let err = prepare_container_root(&prep).unwrap_err();
    assert!(err.to_string().contains("at least one lower layer"));
    assert!(matches!(
        err,
        tokedb_runtime::error::RuntimeError::InvalidConfig(_)
    ));
}
