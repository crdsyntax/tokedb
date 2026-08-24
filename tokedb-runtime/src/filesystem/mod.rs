#[cfg(target_os = "linux")]
pub mod pivot;

pub mod mounts;
pub mod overlay;
pub mod rootfs;

pub use mounts::MountSpec;
pub use overlay::OverlaySpec;
#[cfg(target_os = "linux")]
pub use pivot::pivot_root;
pub use rootfs::{sha256_file, unpack_layer, validate_entry_path};

use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::path::Path;

#[cfg(target_os = "linux")]
use nix::mount::{mount, MsFlags};

#[cfg(target_os = "linux")]
use crate::error::{Result, RuntimeError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootfsPrep {
    pub overlay: OverlaySpec,
    pub bind_mounts: Vec<MountSpec>,
}

#[cfg(target_os = "linux")]
pub fn prepare_container_root(prep: &RootfsPrep) -> Result<()> {
    mount(
        None::<&Path>,
        Path::new("/"),
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .map_err(RuntimeError::from)?;

    overlay::mount_overlay(&prep.overlay)?;
    mounts::mount_devtmpfs(&prep.overlay.target)?;
    mounts::mount_proc(&prep.overlay.target)?;
    mounts::mount_sys(&prep.overlay.target)?;
    create_root_dirs(&prep.overlay.target)?;
    seed_host_etc(&prep.overlay.target)?;
    for mount_spec in &prep.bind_mounts {
        mounts::bind_mount(mount_spec, &prep.overlay.target)?;
    }
    pivot::pivot_root(&prep.overlay.target)
}

/// Ensures a minimal, writable base filesystem exists inside the container
/// rootfs so engines have the standard directories they expect (e.g. `/tmp`,
/// `/run`). Entries are created on the writable overlay upper layer; if the
/// image already provides them this is a no-op.
#[cfg(target_os = "linux")]
fn create_root_dirs(rootfs: &Path) -> Result<()> {
    for (rel, mode) in [("tmp", 0o1777), ("run", 0o755), ("var/run", 0o755)] {
        let dir = rootfs.join(rel);
        fs::create_dir_all(&dir).map_err(|err| RuntimeError::Io {
            path: dir.display().to_string(),
            message: err.to_string(),
        })?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).map_err(|err| {
            RuntimeError::Io {
                path: dir.display().to_string(),
                message: err.to_string(),
            }
        })?;
    }
    Ok(())
}

/// Host `/etc` files that a container needs for basic OS functionality
/// (user/group lookup, name resolution, DNS). These are copied into the
/// container's `/etc` *only if the image does not already provide them*, so an
/// image's own configuration (e.g. `/etc/mysql/my.cnf`) always takes
/// precedence over the host's.
#[cfg(target_os = "linux")]
const HOST_ETC_SEED: &[&str] = &[
    "/etc/passwd",
    "/etc/group",
    "/etc/nsswitch.conf",
    "/etc/resolv.conf",
    "/etc/hosts",
    "/etc/localtime",
    "/etc/ssl/certs",
];

/// Copies the minimal host `/etc` base files into the container rootfs,
/// skipping any path the image already supplies. Runs after the overlay is
/// mounted, so the image's files win on conflict.
#[cfg(target_os = "linux")]
fn seed_host_etc(rootfs: &Path) -> Result<()> {
    let etc = rootfs.join("etc");
    fs::create_dir_all(&etc).map_err(|err| RuntimeError::Io {
        path: etc.display().to_string(),
        message: err.to_string(),
    })?;
    for rel in HOST_ETC_SEED {
        let src = Path::new(rel);
        if !src.exists() {
            continue;
        }
        let dst = rootfs.join(rel.trim_start_matches('/'));
        copy_host_tree(src, &dst)?;
    }
    Ok(())
}

/// Recursive copy used by `seed_host_etc`. If `dst` already exists it is left
/// untouched (so the image-provided file/dir is preserved).
#[cfg(target_os = "linux")]
fn copy_host_tree(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(src).map_err(|err| RuntimeError::Io {
        path: src.display().to_string(),
        message: err.to_string(),
    })?;
    if meta.file_type().is_symlink() {
        let target = fs::read_link(src).map_err(|err| RuntimeError::Io {
            path: src.display().to_string(),
            message: err.to_string(),
        })?;
        std::os::unix::fs::symlink(target, dst).map_err(|err| RuntimeError::Io {
            path: dst.display().to_string(),
            message: err.to_string(),
        })?;
        return Ok(());
    }
    if meta.is_dir() {
        fs::create_dir_all(dst).map_err(|err| RuntimeError::Io {
            path: dst.display().to_string(),
            message: err.to_string(),
        })?;
        for entry in fs::read_dir(src).map_err(|err| RuntimeError::Io {
            path: src.display().to_string(),
            message: err.to_string(),
        })? {
            let entry = entry.map_err(RuntimeError::from)?;
            copy_host_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dst).map_err(|err| RuntimeError::Io {
            path: dst.display().to_string(),
            message: err.to_string(),
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rootfs_prep_serde_roundtrip() {
        let prep = RootfsPrep {
            overlay: OverlaySpec {
                lower_layers: vec![std::path::PathBuf::from("/img/lower")],
                upper_dir: std::path::PathBuf::from("/img/upper"),
                work_dir: std::path::PathBuf::from("/img/work"),
                target: std::path::PathBuf::from("/img/merged"),
            },
            bind_mounts: vec![MountSpec {
                source: std::path::PathBuf::from("/vol/data"),
                target: std::path::PathBuf::from("/var/lib/mysql"),
                read_only: false,
            }],
        };
        let value = serde_json::to_value(&prep).unwrap();
        let decoded: RootfsPrep = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.overlay.lower_layers.len(), 1);
        assert_eq!(decoded.bind_mounts.len(), 1);
        assert_eq!(
            decoded.bind_mounts[0].target,
            std::path::PathBuf::from("/var/lib/mysql")
        );
    }
}
