use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, RuntimeError};
use crate::image::layers::layer_hex;
use crate::image::manifest::read_manifest;
use crate::image::reference::parse_reference;
use crate::image::registry::{verify_manifest_reference, Registry};
use crate::image::storage::{ensure_dir, write_atomic};
use crate::image::Image;

const INDEX_FILE: &str = "index.json";
const BLOBS_DIR: &str = "blobs";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalIndex {
    pub images: Vec<LocalImageRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalImageRef {
    pub reference: String,
    pub manifest_digest: String,
    pub layers: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LocalRegistry {
    root: PathBuf,
}

impl LocalRegistry {
    pub fn new(root: PathBuf) -> Self {
        LocalRegistry { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn list(&self) -> Result<Vec<LocalImageRef>> {
        Ok(self.load_index()?.images)
    }

    pub fn publish(&self, image: &Image) -> Result<()> {
        ensure_dir(&self.root)?;
        let blobs = self.root.join(BLOBS_DIR);
        ensure_dir(&blobs)?;

        let manifest_digest = image.manifest.digest.clone();
        let manifest_blob = blobs.join(format!("{}.json", layer_hex(&manifest_digest)?));
        fs::copy(image.dir.join("manifest.json"), &manifest_blob).map_err(|err| {
            RuntimeError::Io {
                path: manifest_blob.display().to_string(),
                message: err.to_string(),
            }
        })?;

        let mut layer_digests = Vec::new();
        for (layer, path) in image.manifest.layers.iter().zip(&image.layers) {
            let blob = blobs.join(format!("{}.tar.gz", layer_hex(&layer.digest)?));
            fs::copy(path, &blob).map_err(|err| RuntimeError::Io {
                path: blob.display().to_string(),
                message: err.to_string(),
            })?;
            layer_digests.push(layer.digest.clone());
        }

        let mut index = self.load_index()?;
        index
            .images
            .retain(|entry| entry.reference != image.reference);
        index.images.push(LocalImageRef {
            reference: image.reference.clone(),
            manifest_digest,
            layers: layer_digests,
        });
        index.images.sort_by(|a, b| a.reference.cmp(&b.reference));
        self.save_index(&index)
    }

    pub fn has(&self, reference: &str) -> Result<bool> {
        parse_reference(reference)?;
        Ok(self
            .load_index()?
            .images
            .iter()
            .any(|entry| entry.reference == reference))
    }

    pub fn remove(&self, reference: &str) -> Result<()> {
        let mut index = self.load_index()?;
        let before = index.images.len();
        index.images.retain(|entry| entry.reference != reference);
        if index.images.len() == before {
            return Err(RuntimeError::ImageNotFound {
                reference: reference.to_string(),
            });
        }
        self.save_index(&index)
    }

    fn load_index(&self) -> Result<LocalIndex> {
        let path = self.root.join(INDEX_FILE);
        match fs::read_to_string(&path) {
            Ok(raw) => {
                serde_json::from_str(&raw).map_err(|err| RuntimeError::Registry(err.to_string()))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(LocalIndex::default()),
            Err(err) => Err(RuntimeError::Io {
                path: path.display().to_string(),
                message: err.to_string(),
            }),
        }
    }

    fn save_index(&self, index: &LocalIndex) -> Result<()> {
        let payload = serde_json::to_vec_pretty(index).map_err(RuntimeError::from)?;
        write_atomic(&self.root.join(INDEX_FILE), &payload)
    }
}

impl Registry for LocalRegistry {
    fn fetch(&self, reference: &str, staged_dir: &Path) -> Result<()> {
        let (name, tag) = parse_reference(reference)?;
        let entry = self
            .load_index()?
            .images
            .into_iter()
            .find(|entry| entry.reference == reference)
            .ok_or_else(|| RuntimeError::ImageNotFound {
                reference: reference.to_string(),
            })?;

        let blobs = self.root.join(BLOBS_DIR);
        let manifest_blob = blobs.join(format!("{}.json", layer_hex(&entry.manifest_digest)?));
        let manifest = read_manifest(&manifest_blob)?;
        verify_manifest_reference(&manifest, &name, &tag)?;

        ensure_dir(staged_dir)?;
        let layers_dir = staged_dir.join("layers");
        ensure_dir(&layers_dir)?;
        fs::copy(&manifest_blob, staged_dir.join("manifest.json")).map_err(|err| {
            RuntimeError::Io {
                path: staged_dir.join("manifest.json").display().to_string(),
                message: err.to_string(),
            }
        })?;
        for digest in &entry.layers {
            let blob = blobs.join(format!("{}.tar.gz", layer_hex(digest)?));
            let staged_layer = layers_dir.join(format!("{}.tar.gz", layer_hex(digest)?));
            fs::copy(&blob, &staged_layer).map_err(|err| RuntimeError::Io {
                path: staged_layer.display().to_string(),
                message: err.to_string(),
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::manifest::sample_manifest;
    use crate::image::manifest::{ImageManifest, LayerRef};
    use crate::image::storage::ImageStore;
    use std::io::Write;

    fn layer_tar_gz(content: &[u8]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, "db.txt", content).unwrap();
            builder.finish().unwrap();
        }
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn make_image(
        store: &ImageStore,
        database: &str,
        version: &str,
        content: &[u8],
    ) -> crate::image::Image {
        let mut manifest = sample_manifest();
        manifest.database = database.into();
        manifest.version = version.into();
        manifest.digest = String::new();
        let layer_bytes = layer_tar_gz(content);
        let hex = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(&layer_bytes))
        };
        manifest.layers = vec![LayerRef {
            digest: format!("sha256:{hex}"),
            size: layer_bytes.len() as u64,
        }];
        manifest.digest = manifest.compute_digest().unwrap();

        let work = tempfile::tempdir().unwrap();
        let staged = work.path().join("staged");
        std::fs::create_dir_all(staged.join("layers")).unwrap();
        std::fs::write(
            manifest
                .layer_path(&staged.join("layers"), &manifest.layers[0])
                .unwrap(),
            &layer_bytes,
        )
        .unwrap();
        std::fs::write(
            staged.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        store.import_staged(&staged).unwrap()
    }

    #[test]
    fn publish_fetch_roundtrip_pulls_image() {
        let work = tempfile::tempdir().unwrap();
        let store = ImageStore::new(work.path().join("images"));
        let image = make_image(&store, "mariadb", "11.4", b"registry data");
        let registry = LocalRegistry::new(work.path().join("registry"));
        registry.publish(&image).unwrap();
        assert!(registry.has("mariadb:11.4").unwrap());

        let staged = work.path().join("pulled");
        registry.fetch("mariadb:11.4", &staged).unwrap();
        assert!(staged.join("manifest.json").is_file());

        let second = ImageStore::new(work.path().join("images2"));
        let pulled = second.import_staged(&staged).unwrap();
        second.verify(&pulled.reference).unwrap();
        assert_eq!(pulled.manifest.database, "mariadb");
        assert_eq!(pulled.manifest.layers.len(), 1);
    }

    #[test]
    fn publish_overwrites_existing_reference() {
        let work = tempfile::tempdir().unwrap();
        let store = ImageStore::new(work.path().join("images"));
        let image = make_image(&store, "mysql", "9.0", b"v1");
        let registry = LocalRegistry::new(work.path().join("registry"));
        registry.publish(&image).unwrap();
        registry.publish(&image).unwrap();
        let index = registry.load_index().unwrap();
        assert_eq!(index.images.len(), 1);
    }

    #[test]
    fn fetch_unknown_reference_fails_typed() {
        let work = tempfile::tempdir().unwrap();
        let registry = LocalRegistry::new(work.path().join("registry"));
        assert!(!registry.has("mariadb:11").unwrap());
        let staged = work.path().join("pulled");
        std::fs::create_dir_all(&staged).unwrap();
        let err = registry.fetch("mariadb:11", &staged).unwrap_err();
        assert!(matches!(err, RuntimeError::ImageNotFound { .. }));
    }

    #[test]
    fn fetch_then_import_verifies_blob_digests() {
        let work = tempfile::tempdir().unwrap();
        let store = ImageStore::new(work.path().join("images"));
        let image = make_image(&store, "postgres", "17", b"blob data");
        let registry = LocalRegistry::new(work.path().join("registry"));
        registry.publish(&image).unwrap();

        let blob_path = work.path().join("registry/blobs").join(format!(
            "{}.tar.gz",
            layer_hex(&image.manifest.layers[0].digest).unwrap()
        ));
        std::fs::write(&blob_path, b"tampered").unwrap();

        let staged = work.path().join("pulled");
        registry.fetch("postgres:17", &staged).unwrap();
        let second = ImageStore::new(work.path().join("images2"));
        let err = second.import_staged(&staged).unwrap_err();
        assert!(matches!(err, RuntimeError::DigestMismatch { .. }));
        assert!(second.list().unwrap().is_empty());
    }

    #[test]
    fn remove_drops_index_entry() {
        let work = tempfile::tempdir().unwrap();
        let store = ImageStore::new(work.path().join("images"));
        let image = make_image(&store, "mariadb", "11.4", b"x");
        let registry = LocalRegistry::new(work.path().join("registry"));
        registry.publish(&image).unwrap();
        registry.remove("mariadb:11.4").unwrap();
        assert!(!registry.has("mariadb:11.4").unwrap());
        let err = registry.remove("mariadb:11.4").unwrap_err();
        assert!(matches!(err, RuntimeError::ImageNotFound { .. }));
    }

    #[test]
    fn index_serde_roundtrip() {
        let index = LocalIndex {
            images: vec![LocalImageRef {
                reference: "mariadb:11.4".into(),
                manifest_digest: format!("sha256:{}", "ab".repeat(32)),
                layers: vec![format!("sha256:{}", "cd".repeat(32))],
            }],
        };
        let value = serde_json::to_value(&index).unwrap();
        let decoded: LocalIndex = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.images.len(), 1);
        assert_eq!(decoded.images[0].reference, "mariadb:11.4");
    }

    #[test]
    fn import_rejects_reference_mismatch_local() {
        let work = tempfile::tempdir().unwrap();
        let store = ImageStore::new(work.path().join("images"));
        let image = make_image(&store, "mariadb", "11.4", b"x");
        let registry = LocalRegistry::new(work.path().join("registry"));
        registry.publish(&image).unwrap();

        let manifest_path: PathBuf = work.path().join("registry/blobs").join(format!(
            "{}.json",
            layer_hex(&image.manifest.digest).unwrap()
        ));
        let raw = std::fs::read_to_string(&manifest_path).unwrap();
        let mut manifest: ImageManifest = serde_json::from_str(&raw).unwrap();
        manifest.version = "12".into();
        manifest.digest = manifest.compute_digest().unwrap();
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let staged = work.path().join("pulled");
        let err = registry.fetch("mariadb:11.4", &staged).unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidManifest { .. }));
    }
}
