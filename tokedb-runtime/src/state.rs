use std::path::PathBuf;

use crate::config::RuntimeConfig;
use crate::error::{Result, RuntimeError};

pub fn validate_component(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(RuntimeError::InvalidName {
            name: name.to_string(),
            reason: "name must not be empty",
        });
    }
    if name == "." || name == ".." {
        return Err(RuntimeError::InvalidName {
            name: name.to_string(),
            reason: "reserved component",
        });
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(RuntimeError::InvalidName {
            name: name.to_string(),
            reason: "separators are not allowed",
        });
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct StateLayout {
    config: RuntimeConfig,
}

impl StateLayout {
    pub fn new(config: RuntimeConfig) -> Self {
        StateLayout { config }
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn ensure_directories(&self) -> Result<()> {
        for dir in [
            &self.config.data_root,
            &self.config.images_dir,
            &self.config.containers_dir,
            &self.config.volumes_dir,
        ] {
            std::fs::create_dir_all(dir).map_err(|err| RuntimeError::Io {
                path: dir.display().to_string(),
                message: err.to_string(),
            })?;
        }
        Ok(())
    }

    pub fn container_dir(&self, id: &str) -> Result<PathBuf> {
        validate_component(id)?;
        Ok(self.config.containers_dir.join(id))
    }

    pub fn metadata_path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.container_dir(id)?.join("metadata.json"))
    }

    pub fn volume_dir(&self, name: &str) -> Result<PathBuf> {
        validate_component(name)?;
        Ok(self.config.volumes_dir.join(name))
    }

    pub fn image_dir(&self, reference: &str) -> Result<PathBuf> {
        validate_component(reference)?;
        Ok(self.config.images_dir.join(reference))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> StateLayout {
        StateLayout::new(RuntimeConfig::new(PathBuf::from("/tmp/db")))
    }

    #[test]
    fn validate_component_accepts_plain_names() {
        assert!(validate_component("mariadb-prod").is_ok());
        assert!(validate_component("abc123").is_ok());
    }

    #[test]
    fn validate_component_rejects_unsafe_names() {
        assert!(validate_component("").is_err());
        assert!(validate_component(".").is_err());
        assert!(validate_component("..").is_err());
        assert!(validate_component("a/b").is_err());
        assert!(validate_component("a\\b").is_err());
        assert!(validate_component("a\0b").is_err());
    }

    #[test]
    fn state_layout_builds_typed_paths() {
        assert_eq!(
            layout().container_dir("abc123").unwrap(),
            PathBuf::from("/tmp/db/containers/abc123")
        );
        assert_eq!(
            layout().metadata_path("abc123").unwrap(),
            PathBuf::from("/tmp/db/containers/abc123/metadata.json")
        );
        assert_eq!(
            layout().volume_dir("mariadb-prod").unwrap(),
            PathBuf::from("/tmp/db/volumes/mariadb-prod")
        );
        assert_eq!(
            layout().image_dir("mariadb:11").unwrap(),
            PathBuf::from("/tmp/db/images/mariadb:11")
        );
    }

    #[test]
    fn state_layout_rejects_unsafe_ids() {
        assert!(layout().container_dir("..").is_err());
        assert!(layout().volume_dir("a/b").is_err());
    }

    #[test]
    fn ensure_directories_creates_layout() {
        let temp = tempfile::tempdir().unwrap();
        let config = RuntimeConfig::new(temp.path().to_path_buf());
        StateLayout::new(config).ensure_directories().unwrap();
        for entry in ["images", "containers", "volumes"] {
            assert!(temp.path().join(entry).is_dir());
        }
    }
}
