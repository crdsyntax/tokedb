














use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Result, RuntimeError};
use crate::filesystem::unpack_layer;
use crate::image::layer_hex;

const DIFF_DIR: &str = "diff";
const REFCOUNT_FILE: &str = "refcount";

pub struct LayerStore {
    layers_dir: PathBuf,
    
    
    guard: Mutex<()>,
}

impl LayerStore {
    pub fn new(layers_dir: PathBuf) -> Self {
        LayerStore {
            layers_dir,
            guard: Mutex::new(()),
        }
    }

    pub fn layers_dir(&self) -> &Path {
        &self.layers_dir
    }

    fn layer_dir(&self, digest: &str) -> Result<PathBuf> {
        Ok(self.layers_dir.join(layer_hex(digest)?))
    }

    
    
    pub fn diff_path(&self, digest: &str) -> Result<PathBuf> {
        Ok(self.layer_dir(digest)?.join(DIFF_DIR))
    }

    
    
    
    
    pub fn ensure(&self, digest: &str, tar_gz_path: &Path) -> Result<PathBuf> {
        let dir = self.layer_dir(digest)?;
        let diff = dir.join(DIFF_DIR);
        let refcount_path = dir.join(REFCOUNT_FILE);
        let _lock = self
            .guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if diff.exists() {
            let count = read_refcount(&refcount_path)?;
            write_refcount(&refcount_path, count + 1)?;
            return Ok(diff);
        }

        fs::create_dir_all(&dir).map_err(|err| RuntimeError::Io {
            path: dir.display().to_string(),
            message: err.to_string(),
        })?;

        
        
        let staging = dir.join(format!("{DIFF_DIR}.tmp-{}", short_suffix()));
        fs::create_dir_all(&staging).map_err(|err| RuntimeError::Io {
            path: staging.display().to_string(),
            message: err.to_string(),
        })?;
        unpack_layer(tar_gz_path, &staging)?;
        fs::rename(&staging, &diff).map_err(|err| RuntimeError::Io {
            path: diff.display().to_string(),
            message: err.to_string(),
        })?;
        write_refcount(&refcount_path, 1)?;
        Ok(diff)
    }

    
    
    pub fn release(&self, digest: &str) -> Result<()> {
        let dir = match self.layer_dir(digest) {
            Ok(dir) => dir,
            Err(_) => return Ok(()),
        };
        let diff = dir.join(DIFF_DIR);
        let refcount_path = dir.join(REFCOUNT_FILE);
        let _lock = self
            .guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if !diff.exists() {
            return Ok(());
        }
        let count = read_refcount(&refcount_path)?;
        if count <= 1 {
            fs::remove_dir_all(&dir).map_err(|err| RuntimeError::Io {
                path: dir.display().to_string(),
                message: err.to_string(),
            })?;
        } else {
            write_refcount(&refcount_path, count - 1)?;
        }
        Ok(())
    }
}

fn read_refcount(path: &Path) -> Result<u64> {
    match fs::read_to_string(path) {
        Ok(content) => content.trim().parse().map_err(|err| RuntimeError::CorruptState {
            id: path.display().to_string(),
            reason: format!("invalid layer refcount: {err}"),
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        }),
    }
}

fn write_refcount(path: &Path, value: u64) -> Result<()> {
    fs::write(path, value.to_string()).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use flate2::write::GzEncoder;
    use flate2::Compression;

    fn layer_tar_gz(content: &[u8]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "db.txt", content)
                .unwrap();
            builder.finish().unwrap();
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn write_layer_file(dir: &Path, digest: &str, content: &[u8]) -> PathBuf {
        let hex = layer_hex(digest).unwrap();
        let path = dir.join(format!("{hex}.tar.gz"));
        fs::create_dir_all(dir).unwrap();
        fs::write(&path, layer_tar_gz(content)).unwrap();
        path
    }

    #[test]
    fn ensure_unpacks_once_then_shares_and_refcounts() {
        let work = tempfile::tempdir().unwrap();
        let store = LayerStore::new(work.path().join("layers"));
        let digest = format!("sha256:{}", "ab".repeat(32));
        let archive = write_layer_file(work.path(), &digest, b"shared layer");

        let first = store.ensure(&digest, &archive).unwrap();
        assert!(first.join("db.txt").is_file());
        
        let second = store.ensure(&digest, &archive).unwrap();
        assert_eq!(first, second);

        
        
        store.release(&digest).unwrap();
        assert!(first.exists());
        store.release(&digest).unwrap();
        assert!(!first.exists());
    }

    #[test]
    fn release_of_unknown_layer_is_noop() {
        let work = tempfile::tempdir().unwrap();
        let store = LayerStore::new(work.path().join("layers"));
        let digest = format!("sha256:{}", "cd".repeat(32));
        assert!(store.release(&digest).is_ok());
    }

    #[test]
    fn refcount_persists_across_distinct_store_instances() {
        let work = tempfile::tempdir().unwrap();
        let layers_dir = work.path().join("layers");
        let digest = format!("sha256:{}", "ef".repeat(32));
        let archive = write_layer_file(work.path(), &digest, b"persisted layer");

        let store_a = LayerStore::new(layers_dir.clone());
        let first = store_a.ensure(&digest, &archive).unwrap();
        assert!(first.join("db.txt").is_file());

        let hex = crate::image::layer_hex(&digest).unwrap().to_string();
        let refcount_path = layers_dir.join(hex).join("refcount");

        let store_b = LayerStore::new(layers_dir.clone());
        let second = store_b.ensure(&digest, &archive).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            std::fs::read_to_string(&refcount_path).unwrap().trim(),
            "2",
            "a second store instance must see and bump the persisted refcount"
        );

        store_b.release(&digest).unwrap();
        assert!(first.exists());
        store_a.release(&digest).unwrap();
        assert!(!first.exists());
    }
}
