use std::fs;
use std::io::Write;

use uuid::Uuid;

use crate::error::{Result, RuntimeError};
use crate::runtime::container::{Container, ContainerSpec};
use crate::runtime::lifecycle::ContainerState;
use crate::state::{validate_component, StateLayout};

const METADATA_FILE: &str = "metadata.json";

pub struct ContainerStore {
    layout: StateLayout,
}

impl ContainerStore {
    pub fn new(layout: StateLayout) -> Self {
        ContainerStore { layout }
    }

    pub fn create(&self, spec: ContainerSpec) -> Result<Container> {
        validate_component(&spec.name)?;
        if self
            .list()?
            .iter()
            .any(|container| container.name == spec.name)
        {
            return Err(RuntimeError::ContainerAlreadyExists { name: spec.name });
        }

        let id = short_id();
        let container = Container {
            id: id.clone(),
            name: spec.name,
            image: spec.image,
            command: spec.command,
            resources: spec.resources,
            volumes: spec.volumes,
            ports: spec.ports,
            state: ContainerState::Created,
            created_at: now_unix_secs(),
            pid: None,
        };
        let dir = self.layout.container_dir(&id)?;
        fs::create_dir_all(&dir).map_err(|err| RuntimeError::Io {
            path: dir.display().to_string(),
            message: err.to_string(),
        })?;
        self.save(&container)?;
        Ok(container)
    }

    pub fn save(&self, container: &Container) -> Result<()> {
        validate_component(&container.id)?;
        if container.state == ContainerState::Destroyed {
            return Err(RuntimeError::InvalidConfig(
                "cannot persist a destroyed container".into(),
            ));
        }

        let dir = self.layout.container_dir(&container.id)?;
        fs::create_dir_all(&dir).map_err(|err| RuntimeError::Io {
            path: dir.display().to_string(),
            message: err.to_string(),
        })?;

        let payload = serde_json::to_string_pretty(container).map_err(RuntimeError::from)?;
        let final_path = dir.join(METADATA_FILE);
        let tmp_path = dir.join(format!("{}.tmp-{}", METADATA_FILE, short_id()));

        {
            let mut file = fs::File::create(&tmp_path).map_err(|err| RuntimeError::Io {
                path: tmp_path.display().to_string(),
                message: err.to_string(),
            })?;
            file.write_all(payload.as_bytes())
                .map_err(|err| RuntimeError::Io {
                    path: tmp_path.display().to_string(),
                    message: err.to_string(),
                })?;
            file.sync_all().map_err(|err| RuntimeError::Io {
                path: tmp_path.display().to_string(),
                message: err.to_string(),
            })?;
        }
        fs::rename(&tmp_path, &final_path).map_err(|err| RuntimeError::Io {
            path: final_path.display().to_string(),
            message: err.to_string(),
        })?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Container> {
        validate_component(id)?;
        let path = self.layout.metadata_path(id)?;
        match fs::read(&path) {
            Ok(bytes) => {
                let container: Container =
                    serde_json::from_slice(&bytes).map_err(|err| RuntimeError::CorruptState {
                        id: id.to_string(),
                        reason: err.to_string(),
                    })?;
                if container.id != id {
                    return Err(RuntimeError::CorruptState {
                        id: id.to_string(),
                        reason: "id does not match directory".into(),
                    });
                }
                if container.state == ContainerState::Destroyed {
                    return Err(RuntimeError::CorruptState {
                        id: id.to_string(),
                        reason: "destroyed state persisted".into(),
                    });
                }
                Ok(container)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Err(RuntimeError::ContainerNotFound { id: id.to_string() })
            }
            Err(err) => Err(RuntimeError::Io {
                path: path.display().to_string(),
                message: err.to_string(),
            }),
        }
    }

    pub fn list(&self) -> Result<Vec<Container>> {
        let dir = &self.layout.config().containers_dir;
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(RuntimeError::Io {
                    path: dir.display().to_string(),
                    message: err.to_string(),
                })
            }
        };

        let mut containers = Vec::new();
        for entry in entries {
            let entry = entry.map_err(RuntimeError::from)?;
            if entry.file_type().map_err(RuntimeError::from)?.is_dir() {
                let id = entry.file_name().to_string_lossy().into_owned();
                containers.push(self.load(&id)?);
            }
        }
        containers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(containers)
    }

    /// Looks up a container by its (unique) name.
    pub fn find(&self, name: &str) -> Result<Container> {
        validate_component(name)?;
        self.list()?
            .into_iter()
            .find(|container| container.name == name)
            .ok_or_else(|| RuntimeError::ContainerNotFound {
                id: name.to_string(),
            })
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let container = self.load(id)?;
        match container.state {
            ContainerState::Created | ContainerState::Stopped => {}
            _ => return Err(RuntimeError::ContainerNotStopped { id: id.to_string() }),
        }
        let dir = self.layout.container_dir(id)?;
        fs::remove_dir_all(&dir).map_err(|err| RuntimeError::Io {
            path: dir.display().to_string(),
            message: err.to_string(),
        })
    }
}

fn short_id() -> String {
    Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect()
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuntimeConfig;
    use crate::runtime::container::ResourceLimits;
    use crate::runtime::process::CommandSpec;

    fn store() -> (tempfile::TempDir, ContainerStore) {
        let temp = tempfile::tempdir().unwrap();
        let layout = StateLayout::new(RuntimeConfig::new(temp.path().to_path_buf()));
        (temp, ContainerStore::new(layout))
    }

    fn spec(name: &str) -> ContainerSpec {
        ContainerSpec {
            name: name.to_string(),
            image: "mariadb:11".to_string(),
            command: CommandSpec::new("/bin/sh").arg("-c").arg("sleep 1"),
            resources: ResourceLimits {
                memory_bytes: Some(1024),
                cpu_quota: Some(1.5),
                pids_max: Some(42),
            },
            volumes: Vec::new(),
            ports: Vec::new(),
        }
    }

    #[test]
    fn create_persists_metadata() {
        let (_temp, store) = store();
        let container = store.create(spec("mariadb-prod")).unwrap();
        assert_eq!(container.id.len(), 8);
        assert_eq!(container.name, "mariadb-prod");
        assert_eq!(container.state, ContainerState::Created);
        assert!(container.pid.is_none());

        let loaded = store.load(&container.id).unwrap();
        assert_eq!(loaded.name, "mariadb-prod");
        assert_eq!(loaded.image, "mariadb:11");
        assert_eq!(loaded.resources.memory_bytes, Some(1024));
        assert_eq!(loaded.resources.cpu_quota, Some(1.5));
        assert_eq!(loaded.resources.pids_max, Some(42));
    }

    #[test]
    fn create_rejects_duplicate_names() {
        let (_temp, store) = store();
        store.create(spec("dup")).unwrap();
        let err = store.create(spec("dup")).unwrap_err();
        assert!(matches!(err, RuntimeError::ContainerAlreadyExists { ref name } if name == "dup"));
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_files() {
        let (_temp, store) = store();
        let mut container = store.create(spec("atomic")).unwrap();
        container.state = ContainerState::Stopped;
        store.save(&container).unwrap();

        let dir = fs::read_dir(store.layout.container_dir(&container.id).unwrap()).unwrap();
        let files: Vec<_> = dir
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, vec![METADATA_FILE.to_string()]);
        assert!(store.load(&container.id).unwrap().state == ContainerState::Stopped);
    }

    #[test]
    fn save_rejects_destroyed_state() {
        let (_temp, store) = store();
        let mut container = store.create(spec("destroyed")).unwrap();
        container.state = ContainerState::Destroyed;
        let err = store.save(&container).unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidConfig(_)));
    }

    #[test]
    fn load_missing_container_returns_not_found() {
        let (_temp, store) = store();
        let err = store.load("deadbeef").unwrap_err();
        assert!(matches!(err, RuntimeError::ContainerNotFound { ref id } if id == "deadbeef"));
    }

    #[test]
    fn load_rejects_id_mismatch() {
        let (_temp, store) = store();
        let container = store.create(spec("mismatch")).unwrap();
        let foreign_dir = store.layout.container_dir("00000000").unwrap();
        std::fs::create_dir_all(&foreign_dir).unwrap();
        std::fs::write(
            foreign_dir.join(METADATA_FILE),
            serde_json::to_vec(&container).unwrap(),
        )
        .unwrap();
        let err = store.load("00000000").unwrap_err();
        assert!(matches!(err, RuntimeError::CorruptState { .. }));
    }

    #[test]
    fn load_rejects_persisted_destroyed_state() {
        let (_temp, store) = store();
        let mut container = store.create(spec("corrupt")).unwrap();
        container.state = ContainerState::Destroyed;
        std::fs::write(
            store
                .layout
                .container_dir(&container.id)
                .unwrap()
                .join(METADATA_FILE),
            serde_json::to_vec(&container).unwrap(),
        )
        .unwrap();
        let err = store.load(&container.id).unwrap_err();
        assert!(matches!(err, RuntimeError::CorruptState { .. }));
    }

    #[test]
    fn remove_deletes_container_directory() {
        let (_temp, store) = store();
        let container = store.create(spec("removable")).unwrap();
        store.remove(&container.id).unwrap();
        assert!(!store.layout.container_dir(&container.id).unwrap().exists());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn remove_refuses_running_container() {
        let (_temp, store) = store();
        let mut container = store.create(spec("running")).unwrap();
        container.state = ContainerState::Running;
        store.save(&container).unwrap();
        let err = store.remove(&container.id).unwrap_err();
        assert!(matches!(err, RuntimeError::ContainerNotStopped { ref id } if id == &container.id));
    }

    #[test]
    fn list_returns_sorted_containers() {
        let (_temp, store) = store();
        store.create(spec("zeta")).unwrap();
        store.create(spec("alpha")).unwrap();
        let containers = store.list().unwrap();
        let names: Vec<_> = containers.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
    }
}
