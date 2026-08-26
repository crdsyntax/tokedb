use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::RuntimeError;

const DEFAULT_BRIDGE: &str = "db0";
const ENV_DATA_ROOT: &str = "TOKEDB_DATA_ROOT";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub data_root: PathBuf,
    pub images_dir: PathBuf,
    pub layers_dir: PathBuf,
    pub containers_dir: PathBuf,
    pub volumes_dir: PathBuf,
    pub bridge_name: String,
}

impl RuntimeConfig {
    pub fn new(data_root: PathBuf) -> Self {
        let images_dir = data_root.join("images");
        let layers_dir = data_root.join("layers");
        let containers_dir = data_root.join("containers");
        let volumes_dir = data_root.join("volumes");
        RuntimeConfig {
            data_root,
            images_dir,
            layers_dir,
            containers_dir,
            volumes_dir,
            bridge_name: DEFAULT_BRIDGE.to_string(),
        }
    }

    pub fn from_env() -> Result<Self, RuntimeError> {
        match std::env::var(ENV_DATA_ROOT) {
            Ok(raw) if !raw.trim().is_empty() => Ok(RuntimeConfig::new(PathBuf::from(raw))),
            Ok(_) => Err(RuntimeError::InvalidConfig(format!(
                "{} must not be empty",
                ENV_DATA_ROOT
            ))),
            Err(_) => Ok(RuntimeConfig::default()),
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        RuntimeConfig::new(default_data_root())
    }
}

fn default_data_root() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/var/lib/db-runtime")
    }
    #[cfg(not(target_os = "linux"))]
    {
        PathBuf::from(".db-runtime")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn config_derives_known_subdirectories() {
        let config = RuntimeConfig::new(PathBuf::from("/tmp/db"));
        assert_eq!(config.images_dir, PathBuf::from("/tmp/db/images"));
        assert_eq!(config.containers_dir, PathBuf::from("/tmp/db/containers"));
        assert_eq!(config.volumes_dir, PathBuf::from("/tmp/db/volumes"));
        assert_eq!(config.bridge_name, "db0");
    }

    #[test]
    fn config_from_env_overrides_data_root() {
        let _guard = lock_env();
        std::env::set_var(ENV_DATA_ROOT, "/tmp/db-env");
        let config = RuntimeConfig::from_env().unwrap();
        assert_eq!(config.data_root, PathBuf::from("/tmp/db-env"));
        std::env::remove_var(ENV_DATA_ROOT);
    }

    #[test]
    fn config_from_env_rejects_empty_root() {
        let _guard = lock_env();
        std::env::set_var(ENV_DATA_ROOT, "   ");
        let err = RuntimeConfig::from_env().unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidConfig(_)));
        std::env::remove_var(ENV_DATA_ROOT);
    }

    #[test]
    fn config_from_env_falls_back_to_default() {
        let _guard = lock_env();
        std::env::remove_var(ENV_DATA_ROOT);
        assert_eq!(
            RuntimeConfig::from_env().unwrap().data_root,
            default_data_root()
        );
    }
}
