use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::path::Path;

#[cfg(target_os = "linux")]
use nix::mount::{mount, MsFlags};

#[cfg(target_os = "linux")]
use crate::error::{Result, RuntimeError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountSpec {
    pub source: PathBuf,
    pub target: PathBuf,
    pub read_only: bool,
}

#[cfg(target_os = "linux")]
pub fn mount_devtmpfs(rootfs: &Path) -> Result<()> {
    let dev = rootfs.join("dev");
    fs::create_dir_all(&dev).map_err(|err| RuntimeError::Io {
        path: dev.display().to_string(),
        message: err.to_string(),
    })?;

    mount(
        Some("devtmpfs"),
        &dev,
        Some("devtmpfs"),
        MsFlags::empty(),
        None::<&str>,
    )
    .map_err(RuntimeError::from)?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn mount_proc(rootfs: &Path) -> Result<()> {
    let proc = rootfs.join("proc");
    fs::create_dir_all(&proc).map_err(|err| RuntimeError::Io {
        path: proc.display().to_string(),
        message: err.to_string(),
    })?;

    mount(
        Some("proc"),
        &proc,
        Some("proc"),
        MsFlags::empty(),
        None::<&str>,
    )
    .map_err(RuntimeError::from)?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn mount_sys(rootfs: &Path) -> Result<()> {
    let sys = rootfs.join("sys");
    fs::create_dir_all(&sys).map_err(|err| RuntimeError::Io {
        path: sys.display().to_string(),
        message: err.to_string(),
    })?;

    mount(
        Some("sysfs"),
        &sys,
        Some("sysfs"),
        MsFlags::empty(),
        None::<&str>,
    )
    .map_err(RuntimeError::from)?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn bind_mount(spec: &MountSpec, rootfs: &Path) -> Result<()> {
    let relative = spec.target.strip_prefix("/").unwrap_or(&spec.target);
    let target = rootfs.join(relative);

    fs::create_dir_all(&target).map_err(|err| RuntimeError::Io {
        path: target.display().to_string(),
        message: err.to_string(),
    })?;

    mount(
        Some(&spec.source),
        &target,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .map_err(RuntimeError::from)?;

    if spec.read_only {
        mount(
            Some(&spec.source),
            &target,
            None::<&str>,
            MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY | MsFlags::MS_REC,
            None::<&str>,
        )
        .map_err(RuntimeError::from)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_spec_serde_roundtrip() {
        let spec = MountSpec {
            source: PathBuf::from("/var/lib/db-runtime/volumes/data"),
            target: PathBuf::from("/var/lib/mysql"),
            read_only: false,
        };
        let value = serde_json::to_value(&spec).unwrap();
        let decoded: MountSpec = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.source, spec.source);
        assert_eq!(decoded.target, spec.target);
        assert!(!decoded.read_only);
    }
}
