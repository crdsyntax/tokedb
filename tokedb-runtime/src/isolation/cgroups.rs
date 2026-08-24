use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Result, RuntimeError};
use crate::state::validate_component;

const CPU_PERIOD_US: u64 = 100_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub cpu_quota: Option<f64>,
    pub memory_bytes: Option<u64>,
    pub pids_max: Option<u64>,
}

impl ResourceLimits {
    pub fn validate(&self) -> Result<()> {
        if let Some(quota) = self.cpu_quota {
            if quota.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                return Err(RuntimeError::InvalidConfig(format!(
                    "cpu_quota must be positive, got {quota}"
                )));
            }
        }
        if let Some(bytes) = self.memory_bytes {
            if bytes == 0 {
                return Err(RuntimeError::InvalidConfig(
                    "memory_bytes must be greater than zero".into(),
                ));
            }
        }
        if let Some(max) = self.pids_max {
            if max == 0 {
                return Err(RuntimeError::InvalidConfig(
                    "pids_max must be greater than zero".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn cpu_max_line(&self) -> Option<String> {
        self.cpu_quota.map(|quota| {
            let us = (quota * CPU_PERIOD_US as f64).round() as u64;
            format!("{us} {CPU_PERIOD_US}")
        })
    }
}

#[derive(Debug, Clone)]
pub struct CgroupManager {
    base: PathBuf,
}

impl CgroupManager {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        CgroupManager { base: base.into() }
    }

    pub fn create(&self, name: &str) -> Result<()> {
        validate_component(name)?;
        fs::create_dir_all(&self.base).map_err(|err| RuntimeError::Io {
            path: self.base.display().to_string(),
            message: err.to_string(),
        })?;
        self.enable_base_controllers()?;
        let dir = self.dir(name);
        fs::create_dir_all(&dir).map_err(|err| RuntimeError::Io {
            path: dir.display().to_string(),
            message: err.to_string(),
        })
    }

    fn enable_base_controllers(&self) -> Result<()> {
        let controllers =
            fs::read_to_string(self.base.join("cgroup.controllers")).map_err(|err| {
                RuntimeError::CgroupWrite {
                    file: self.base.join("cgroup.controllers").display().to_string(),
                    message: err.to_string(),
                }
            })?;
        let enabled =
            fs::read_to_string(self.base.join("cgroup.subtree_control")).unwrap_or_default();
        let pending = controllers
            .split_whitespace()
            .filter(|controller| {
                !enabled
                    .split_whitespace()
                    .any(|active| active == *controller)
            })
            .map(|controller| format!("+{controller}"))
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return Ok(());
        }
        let target = self.base.join("cgroup.subtree_control");
        fs::write(&target, pending.join(" ")).map_err(|err| RuntimeError::CgroupWrite {
            file: target.display().to_string(),
            message: err.to_string(),
        })
    }

    pub fn apply(&self, name: &str, limits: &ResourceLimits) -> Result<()> {
        limits.validate()?;
        validate_component(name)?;
        if let Some(line) = limits.cpu_max_line() {
            self.write(name, "cpu.max", line)?;
        }
        if let Some(bytes) = limits.memory_bytes {
            self.write(name, "memory.max", bytes.to_string())?;
            let swap = self.dir(name).join("memory.swap.max");
            if swap.exists() {
                self.write(name, "memory.swap.max", bytes.to_string())?;
            }
        }
        if let Some(max) = limits.pids_max {
            self.write(name, "pids.max", max.to_string())?;
        }
        Ok(())
    }

    pub fn attach(&self, name: &str, pid: u32) -> Result<()> {
        validate_component(name)?;
        self.write(name, "cgroup.procs", pid.to_string())
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        validate_component(name)?;
        let dir = self.dir(name);
        let kill_file = dir.join("cgroup.kill");
        if kill_file.exists() {
            fs::write(&kill_file, "1").map_err(|err| RuntimeError::CgroupWrite {
                file: kill_file.display().to_string(),
                message: err.to_string(),
            })?;
        }
        fs::remove_dir(&dir).map_err(|err| RuntimeError::Io {
            path: dir.display().to_string(),
            message: err.to_string(),
        })
    }

    fn write(&self, name: &str, file: &str, value: String) -> Result<()> {
        let path = self.dir(name).join(file);
        fs::write(&path, value).map_err(|err| RuntimeError::CgroupWrite {
            file: path.display().to_string(),
            message: err.to_string(),
        })
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.base.join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_limits_serde_roundtrip() {
        let limits = ResourceLimits {
            cpu_quota: Some(0.5),
            memory_bytes: Some(512 * 1024 * 1024),
            pids_max: Some(128),
        };
        let value = serde_json::to_value(limits).unwrap();
        let decoded: ResourceLimits = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, limits);
    }

    #[test]
    fn resource_limits_defaults_to_unlimited() {
        let limits = ResourceLimits::default();
        assert!(limits.validate().is_ok());
        assert_eq!(limits.cpu_max_line(), None);
    }

    #[test]
    fn resource_limits_rejects_zero_or_negative_values() {
        let zero_memory = ResourceLimits {
            memory_bytes: Some(0),
            ..ResourceLimits::default()
        };
        assert!(zero_memory.validate().is_err());

        let zero_pids = ResourceLimits {
            pids_max: Some(0),
            ..ResourceLimits::default()
        };
        assert!(zero_pids.validate().is_err());

        let negative_cpu = ResourceLimits {
            cpu_quota: Some(-1.0),
            ..ResourceLimits::default()
        };
        assert!(negative_cpu.validate().is_err());

        let zero_cpu = ResourceLimits {
            cpu_quota: Some(0.0),
            ..ResourceLimits::default()
        };
        assert!(zero_cpu.validate().is_err());
    }

    #[test]
    fn cpu_max_line_formats_quota_fraction() {
        let half = ResourceLimits {
            cpu_quota: Some(0.5),
            ..ResourceLimits::default()
        };
        assert_eq!(half.cpu_max_line().unwrap(), "50000 100000");

        let two_cores = ResourceLimits {
            cpu_quota: Some(2.0),
            ..ResourceLimits::default()
        };
        assert_eq!(two_cores.cpu_max_line().unwrap(), "200000 100000");
    }

    #[test]
    fn cgroup_manager_rejects_unsafe_names() {
        let manager = CgroupManager::new("/sys/fs/cgroup/tokedb");
        assert!(manager.create("a/b").is_err());
        assert!(manager.create("..").is_err());
        assert!(manager.create("").is_err());
    }
}
