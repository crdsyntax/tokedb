use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, RuntimeError};
use crate::state::validate_component;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    pub name: String,
    pub path: PathBuf,
}

impl Volume {
    pub fn mount_spec(&self, target: PathBuf, read_only: bool) -> crate::filesystem::MountSpec {
        crate::filesystem::MountSpec {
            source: self.path.clone(),
            target,
            read_only,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VolumeStore {
    volumes_dir: PathBuf,
}

impl VolumeStore {
    pub fn new(volumes_dir: PathBuf) -> Self {
        VolumeStore { volumes_dir }
    }

    pub fn volumes_dir(&self) -> &Path {
        &self.volumes_dir
    }

    fn volume_path(&self, name: &str) -> Result<PathBuf> {
        validate_component(name)?;
        Ok(self.volumes_dir.join(name))
    }

    pub fn create(&self, name: &str) -> Result<Volume> {
        let path = self.volume_path(name)?;
        fs::create_dir_all(&self.volumes_dir).map_err(|err| RuntimeError::Io {
            path: self.volumes_dir.display().to_string(),
            message: err.to_string(),
        })?;
        create_dir_if_missing(&path)?;
        if !path.is_dir() {
            return Err(RuntimeError::Io {
                path: path.display().to_string(),
                message: "exists but is not a directory".to_string(),
            });
        }
        if !volume_marker_exists(&path) {
            write_volume_marker(&path)?;
        }
        Ok(Volume {
            name: name.to_string(),
            path,
        })
    }

    pub fn get(&self, name: &str) -> Result<Volume> {
        let path = self.volume_path(name)?;
        if !path.is_dir() || !volume_marker_exists(&path) {
            return Err(RuntimeError::VolumeNotFound {
                name: name.to_string(),
            });
        }
        Ok(Volume {
            name: name.to_string(),
            path,
        })
    }

    pub fn list(&self) -> Result<Vec<Volume>> {
        if !self.volumes_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut volumes = Vec::new();
        let entries = fs::read_dir(&self.volumes_dir).map_err(|err| RuntimeError::Io {
            path: self.volumes_dir.display().to_string(),
            message: err.to_string(),
        })?;
        for entry in entries {
            let entry = entry.map_err(|err| RuntimeError::Io {
                path: self.volumes_dir.display().to_string(),
                message: err.to_string(),
            })?;
            let path = entry.path();
            if path.is_dir() && volume_marker_exists(&path) {
                if let Some(name) = path.file_name().and_then(|oss| oss.to_str()) {
                    volumes.push(Volume {
                        name: name.to_string(),
                        path,
                    });
                }
            }
        }
        volumes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(volumes)
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let path = self.volume_path(name)?;
        if !path.exists() {
            return Err(RuntimeError::VolumeNotFound {
                name: name.to_string(),
            });
        }
        if !path.is_dir() {
            return Err(RuntimeError::Io {
                path: path.display().to_string(),
                message: "not a directory".to_string(),
            });
        }
        fs::remove_dir_all(&path).map_err(|err| RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })
    }
}

const VOLUME_MARKER: &str = ".tokedb-volume";

fn create_dir_if_missing(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        }),
    }
}

fn volume_marker_exists(path: &Path) -> bool {
    path.join(VOLUME_MARKER).is_file()
}

fn write_volume_marker(path: &Path) -> Result<()> {
    fs::write(path.join(VOLUME_MARKER), b"").map_err(|err| RuntimeError::Io {
        path: path.join(VOLUME_MARKER).display().to_string(),
        message: err.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_places_volume_under_volumes_dir() {
        let work = tempfile::tempdir().unwrap();
        let store = VolumeStore::new(work.path().join("volumes"));
        let volume = store.create("data").unwrap();
        assert_eq!(volume.name, "data");
        assert_eq!(volume.path, work.path().join("volumes").join("data"));
        assert!(volume.path.is_dir());
        assert!(volume.path.join(VOLUME_MARKER).is_file());
        assert!(store.get("data").unwrap().path.is_dir());
    }

    #[test]
    fn create_is_idempotent() {
        let work = tempfile::tempdir().unwrap();
        let store = VolumeStore::new(work.path().join("volumes"));
        store.create("data").unwrap();
        store.create("data").unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn list_returns_sorted_volumes() {
        let work = tempfile::tempdir().unwrap();
        let store = VolumeStore::new(work.path().join("volumes"));
        store.create("zebra").unwrap();
        store.create("alpha").unwrap();
        let names: Vec<String> = store.list().unwrap().into_iter().map(|v| v.name).collect();
        assert_eq!(names, vec!["alpha".to_string(), "zebra".to_string()]);
    }

    #[test]
    fn list_ignores_non_volume_directories() {
        let work = tempfile::tempdir().unwrap();
        let store = VolumeStore::new(work.path().join("volumes"));
        store.create("data").unwrap();
        fs::create_dir_all(work.path().join("volumes").join("scratch")).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn remove_deletes_volume_and_reports_missing() {
        let work = tempfile::tempdir().unwrap();
        let store = VolumeStore::new(work.path().join("volumes"));
        store.create("data").unwrap();
        store.remove("data").unwrap();
        assert!(!work.path().join("volumes").join("data").exists());
        assert!(matches!(
            store.remove("data"),
            Err(RuntimeError::VolumeNotFound { ref name }) if name == "data"
        ));
    }

    #[test]
    fn rejects_unsafe_names() {
        let work = tempfile::tempdir().unwrap();
        let store = VolumeStore::new(work.path().join("volumes"));
        for bad in ["..", ".", "a/b", "a\\b", "a\0b", ""] {
            assert!(store.create(bad).is_err(), "name `{bad}` must be rejected");
            assert!(store.get(bad).is_err(), "name `{bad}` must be rejected");
            assert!(store.remove(bad).is_err(), "name `{bad}` must be rejected");
        }
    }

    #[test]
    fn mount_spec_points_at_volume_path() {
        let work = tempfile::tempdir().unwrap();
        let store = VolumeStore::new(work.path().join("volumes"));
        let volume = store.create("data").unwrap();
        let spec = volume.mount_spec(PathBuf::from("/var/lib/mysql"), true);
        assert_eq!(spec.source, volume.path);
        assert_eq!(spec.target, PathBuf::from("/var/lib/mysql"));
        assert!(spec.read_only);
    }

    #[test]
    fn get_rejects_missing_volume() {
        let work = tempfile::tempdir().unwrap();
        let store = VolumeStore::new(work.path().join("volumes"));
        assert!(matches!(
            store.get("nope"),
            Err(RuntimeError::VolumeNotFound { ref name }) if name == "nope"
        ));
    }
}
