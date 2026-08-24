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
pub struct OverlaySpec {
    pub lower_layers: Vec<PathBuf>,
    pub upper_dir: PathBuf,
    pub work_dir: PathBuf,
    pub target: PathBuf,
}

#[cfg(target_os = "linux")]
pub fn mount_overlay(spec: &OverlaySpec) -> Result<()> {
    if spec.lower_layers.is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "overlay requires at least one lower layer".into(),
        ));
    }

    for dir in [&spec.upper_dir, &spec.work_dir, &spec.target] {
        fs::create_dir_all(dir).map_err(|err| RuntimeError::Io {
            path: dir.display().to_string(),
            message: err.to_string(),
        })?;
    }

    let lowerdir = spec
        .lower_layers
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":");
    let data = format!(
        "lowerdir={lowerdir},upperdir={},workdir={}",
        spec.upper_dir.display(),
        spec.work_dir.display()
    );

    mount(
        None::<&Path>,
        &spec.target,
        Some("overlay"),
        MsFlags::empty(),
        Some(data.as_str()),
    )
    .map_err(RuntimeError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_spec_serde_roundtrip() {
        let spec = OverlaySpec {
            lower_layers: vec![PathBuf::from("/img/layer-1"), PathBuf::from("/img/layer-2")],
            upper_dir: PathBuf::from("/img/upper"),
            work_dir: PathBuf::from("/img/work"),
            target: PathBuf::from("/img/merged"),
        };
        let value = serde_json::to_value(&spec).unwrap();
        let decoded: OverlaySpec = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.lower_layers.len(), 2);
        assert_eq!(decoded.target, PathBuf::from("/img/merged"));
    }
}
