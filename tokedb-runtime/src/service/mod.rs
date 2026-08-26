use std::path::{Path, PathBuf};

use crate::config::RuntimeConfig;
use crate::error::{Result, RuntimeError};
use crate::image::registry::{LocalImageRef, LocalRegistry, Registry, RemoteRegistry};
use crate::image::{Image, ImageStore, ImageSummary};
use crate::runtime::{
    metrics, run, Container, ContainerLogs, ContainerSpec, ContainerState, ContainerStore,
    ResourceLimits, ResourceUsage, VolumeMount,
};

pub use crate::runtime::container::DbUser;
use crate::state::{validate_component, StateLayout};
use crate::storage::{backup_volume, LayerStore, Volume, VolumeStore};

#[derive(Debug, Clone, Default)]
pub struct CreateRequest {
    pub name: String,
    pub image: String,
    pub resources: ResourceLimits,
    pub ports: Vec<String>,
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
    
    
    pub db_user: DbUser,
}

#[derive(Clone)]
pub struct RuntimeService {
    config: RuntimeConfig,
}

impl RuntimeService {
    pub fn new(config: RuntimeConfig) -> Self {
        RuntimeService { config }
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    fn store(&self) -> ImageStore {
        ImageStore::new(self.config.images_dir.clone())
    }

    fn containers(&self) -> ContainerStore {
        ContainerStore::new(StateLayout::new(self.config.clone()))
    }

    fn volumes(&self) -> VolumeStore {
        VolumeStore::new(self.config.volumes_dir.clone())
    }

    fn layout(&self) -> StateLayout {
        StateLayout::new(self.config.clone())
    }

    fn layer_store(&self) -> LayerStore {
        LayerStore::new(self.config.layers_dir.clone())
    }

    pub fn pull(&self, reference: &str, registry: Option<&str>) -> Result<Image> {
        validate_component(reference)?;
        let registry: Box<dyn Registry> = match registry {
            None => Box::new(LocalRegistry::new(self.config.data_root.join("registry"))),
            Some(source) if source.starts_with("http://") || source.starts_with("https://") => {
                Box::new(RemoteRegistry::new(source.to_string())?)
            }
            Some(source) if source.contains("://") => {
                return Err(RuntimeError::InvalidConfig(format!(
                    "unsupported registry scheme in `{source}`"
                )))
            }
            Some(path) => Box::new(LocalRegistry::new(PathBuf::from(path))),
        };

        let store = self.store();
        let staged = staging_dir(&store)?;
        let outcome = (|| {
            registry.fetch(reference, &staged)?;
            store.import_staged(&staged)
        })();
        if outcome.is_err() {
            let _ = std::fs::remove_dir_all(&staged);
        }
        outcome
    }

    pub fn import(&self, path: &Path) -> Result<Image> {
        self.store().import_bundle(path)
    }

    pub fn export(&self, reference: &str, output: &Path) -> Result<()> {
        validate_component(reference)?;
        self.store().export_bundle(reference, output)
    }

    pub fn images(&self) -> Result<Vec<ImageSummary>> {
        self.store().list()
    }

    pub fn remove_image(&self, reference: &str) -> Result<()> {
        validate_component(reference)?;
        let containers = self.containers();
        for container in containers.list()? {
            if container.image == reference {
                return Err(RuntimeError::ImageInUse {
                    reference: reference.to_string(),
                });
            }
        }
        self.store().remove(reference)
    }

    pub fn create(&self, request: &CreateRequest) -> Result<Container> {
        validate_component(&request.name)?;
        validate_component(&request.image)?;

        let db_user = request.db_user.clone();
        if db_user.username.trim().is_empty() || db_user.password.is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "el usuario y la contraseña de la base de datos son obligatorios al crear el contenedor".into(),
            ));
        }

        let image = self.store().get(&request.image)?;

        let mut ports = Vec::with_capacity(request.ports.len());
        for raw in &request.ports {
            ports.push(run::parse_port_binding(raw)?);
        }

        let volume_name = format!("{}-data", request.name);
        let mut command = run::command_from_image(&image.manifest);
        for (key, value) in &request.env {
            command = command.env(key.clone(), value.clone());
        }
        for arg in &request.args {
            command = command.arg(arg.clone());
        }
        let container = self.containers().create(ContainerSpec {
            name: request.name.clone(),
            image: request.image.clone(),
            command,
            resources: request.resources,
            volumes: vec![VolumeMount {
                name: volume_name.clone(),
                mount_path: image.manifest.data_directory.clone(),
            }],
            ports,
            db_user,
        })?;

        if let Err(err) = self.volumes().create(&volume_name) {
            let _ = self.containers().remove(&container.id);
            return Err(err);
        }
        Ok(container)
    }

    pub fn start(&self, name: &str) -> Result<()> {
        validate_component(name)?;
        run::start(
            &self.containers(),
            &self.store(),
            &self.volumes(),
            &self.layer_store(),
            &self.layout(),
            name,
        )
    }

    pub fn stop(&self, name: &str) -> Result<()> {
        validate_component(name)?;
        run::stop(&self.containers(), name)
    }

    pub fn logs(&self, name: &str) -> Result<()> {
        validate_component(name)?;
        run::logs(&self.containers(), &self.layout(), name)
    }

    pub fn read_logs(&self, name: &str) -> Result<ContainerLogs> {
        validate_component(name)?;
        run::read_logs(&self.containers(), &self.layout(), name)
    }

    pub fn inspect(&self, name: &str) -> Result<Container> {
        validate_component(name)?;
        self.containers().find(name)
    }

    pub fn destroy(&self, name: &str) -> Result<()> {
        validate_component(name)?;
        let containers = self.containers();
        let container = containers.find(name)?;
        let layers = self.layer_store();
        for digest in &container.acquired_layers {
            layers.release(digest)?;
        }
        containers.remove(&container.id)
    }

    pub fn list(&self) -> Result<Vec<Container>> {
        self.containers().list()
    }

    pub fn stats(&self, name: &str) -> Result<ResourceUsage> {
        validate_component(name)?;
        let container = self.containers().find(name)?;
        let memory_limit = container.resources.memory_bytes;
        match container.pid {
            Some(pid) if container.state == ContainerState::Running => {
                Ok(metrics::collect_usage(pid, memory_limit))
            }
            _ => Ok(ResourceUsage::default()),
        }
    }

    pub fn volume_list(&self) -> Result<Vec<Volume>> {
        self.volumes().list()
    }

    pub fn volume_create(&self, name: &str) -> Result<Volume> {
        self.volumes().create(name)
    }

    pub fn volume_remove(&self, name: &str) -> Result<()> {
        self.volumes().remove(name)
    }

    pub fn volume_backup(&self, name: &str, dest_dir: &Path) -> Result<PathBuf> {
        let volume = self.volumes().get(name)?;
        backup_volume(&volume, dest_dir)
    }

    pub fn registry_list(&self) -> Result<Vec<LocalImageRef>> {
        LocalRegistry::new(self.config.data_root.join("registry")).list()
    }

    pub fn registry_publish(&self, reference: &str, registry: Option<&str>) -> Result<()> {
        let image = self.store().get(reference)?;
        let registry = match registry {
            None => LocalRegistry::new(self.config.data_root.join("registry")),
            Some(path) => LocalRegistry::new(PathBuf::from(path)),
        };
        registry.publish(&image)
    }
}

fn staging_dir(store: &ImageStore) -> Result<PathBuf> {
    let path = store.images_dir().join(format!(".pull-{}", short_suffix()));
    std::fs::create_dir_all(&path).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    Ok(path)
}

fn short_suffix() -> String {
    use uuid::Uuid;
    Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect()
}
