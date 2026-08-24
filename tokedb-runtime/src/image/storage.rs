use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

use crate::database::for_engine;
use crate::error::{Result, RuntimeError};
use crate::filesystem::unpack_layer;
use crate::image::layers::verify_digest;
use crate::image::manifest::load_manifest;
use crate::image::manifest::{Architecture, ImageManifest, LayerRef};
use crate::image::reference::{join_reference, parse_reference};

const MANIFEST_FILE: &str = "manifest.json";
const LAYERS_DIR: &str = "layers";
const STAGED_PREFIX: &str = ".tmp-";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSummary {
    pub reference: String,
    pub database: String,
    pub version: String,
    pub architecture: Architecture,
    pub digest: String,
    pub layer_count: usize,
}

#[derive(Debug, Clone)]
pub struct Image {
    pub reference: String,
    pub dir: PathBuf,
    pub manifest: ImageManifest,
    pub layers: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ImageStore {
    images_dir: PathBuf,
}

impl ImageStore {
    pub fn new(images_dir: PathBuf) -> Self {
        ImageStore { images_dir }
    }

    pub fn images_dir(&self) -> &Path {
        &self.images_dir
    }

    pub fn import_bundle(&self, archive: &Path) -> Result<Image> {
        self.ensure_images_dir()?;
        let staged = self.stage_dir()?;
        unpack_layer(archive, &staged)?;
        self.import_staged(&staged)
    }

    pub fn import_staged(&self, staged: &Path) -> Result<Image> {
        self.ensure_images_dir()?;
        let manifest = load_manifest(&staged.join(MANIFEST_FILE), &staged.display().to_string())?;
        if for_engine(&manifest.database).is_none() {
            return Err(RuntimeError::InvalidManifest {
                reason: format!(
                    "unsupported database engine `{}` (expected one of: mariadb, mysql, postgres, mongodb)",
                    manifest.database
                ),
            });
        }
        manifest.verify_digest()?;
        manifest.verify_layers_in_dir(&staged.join(LAYERS_DIR))?;
        let reference = join_reference(&manifest.database, &manifest.version);
        let final_dir = self.image_dir(&reference)?;
        if final_dir.exists() {
            return Err(RuntimeError::ImageAlreadyExists { reference });
        }
        let parent = final_dir
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(self.images_dir.as_path());
        fs::create_dir_all(parent).map_err(|err| RuntimeError::Io {
            path: parent.display().to_string(),
            message: err.to_string(),
        })?;
        fs::rename(staged, &final_dir).map_err(|err| RuntimeError::Io {
            path: final_dir.display().to_string(),
            message: err.to_string(),
        })?;
        self.load_from(&final_dir, &reference)
    }

    pub fn export_bundle(&self, reference: &str, dest: &Path) -> Result<()> {
        let image = self.get(reference)?;
        self.verify(reference)?;

        let parent = dest
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|err| RuntimeError::Io {
            path: parent.display().to_string(),
            message: err.to_string(),
        })?;
        let tmp_path = parent.join(format!(
            "{}.tmp-{}",
            dest.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "bundle".into()),
            short_suffix()
        ));

        let result = write_bundle(&image, &tmp_path);
        if result.is_ok() {
            fs::rename(&tmp_path, dest).map_err(|err| RuntimeError::Io {
                path: dest.display().to_string(),
                message: err.to_string(),
            })?;
        } else {
            let _ = fs::remove_file(&tmp_path);
        }
        result
    }

    pub fn get(&self, reference: &str) -> Result<Image> {
        let dir = self.image_dir(reference)?;
        if !dir.is_dir() {
            return Err(RuntimeError::ImageNotFound {
                reference: reference.to_string(),
            });
        }
        self.load_from(&dir, reference)
    }

    pub fn verify(&self, reference: &str) -> Result<()> {
        let image = self.get(reference)?;
        image.manifest.verify_digest().map_err(|err| match err {
            RuntimeError::DigestMismatch { expected, actual } => RuntimeError::CorruptImage {
                reference: reference.to_string(),
                reason: format!("manifest digest mismatch: expected {expected}, got {actual}"),
            },
            other => other,
        })?;
        image
            .manifest
            .verify_layers_in_dir(&image.dir.join(LAYERS_DIR))
            .map_err(|err| match err {
                RuntimeError::DigestMismatch { expected, actual } => RuntimeError::CorruptImage {
                    reference: reference.to_string(),
                    reason: format!("layer digest mismatch: expected {expected}, got {actual}"),
                },
                RuntimeError::InvalidManifest { reason } => RuntimeError::CorruptImage {
                    reference: reference.to_string(),
                    reason,
                },
                other => other,
            })
    }

    pub fn remove(&self, reference: &str) -> Result<()> {
        let dir = self.image_dir(reference)?;
        if !dir.is_dir() {
            return Err(RuntimeError::ImageNotFound {
                reference: reference.to_string(),
            });
        }
        fs::remove_dir_all(&dir).map_err(|err| RuntimeError::Io {
            path: dir.display().to_string(),
            message: err.to_string(),
        })
    }

    pub fn list(&self) -> Result<Vec<ImageSummary>> {
        let entries = match fs::read_dir(&self.images_dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(RuntimeError::Io {
                    path: self.images_dir.display().to_string(),
                    message: err.to_string(),
                })
            }
        };

        let mut summaries = Vec::new();
        for entry in entries {
            let entry = entry.map_err(RuntimeError::from)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !entry.file_type().map_err(RuntimeError::from)?.is_dir() {
                continue;
            }
            let tags = match fs::read_dir(entry.path()) {
                Ok(tags) => tags,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(RuntimeError::Io {
                        path: entry.path().display().to_string(),
                        message: err.to_string(),
                    })
                }
            };
            for tag_entry in tags {
                let tag_entry = tag_entry.map_err(RuntimeError::from)?;
                let tag = tag_entry.file_name().to_string_lossy().into_owned();
                if tag.starts_with('.')
                    || !tag_entry.file_type().map_err(RuntimeError::from)?.is_dir()
                {
                    continue;
                }
                let reference = join_reference(&name, &tag);
                let manifest = load_manifest(&tag_entry.path().join(MANIFEST_FILE), &reference)?;
                summaries.push(summarize(&reference, &manifest));
            }
        }
        summaries.sort_by(|a, b| a.reference.cmp(&b.reference));
        Ok(summaries)
    }

    pub fn image_dir(&self, reference: &str) -> Result<PathBuf> {
        let (name, tag) = parse_reference(reference)?;
        Ok(self.images_dir.join(name).join(tag))
    }

    fn load_from(&self, dir: &Path, reference: &str) -> Result<Image> {
        let manifest = load_manifest(&dir.join(MANIFEST_FILE), reference)?;
        let layers_dir = dir.join(LAYERS_DIR);
        let mut layers = Vec::new();
        for layer in &manifest.layers {
            let path = manifest.layer_path(&layers_dir, layer)?;
            if !path.is_file() {
                return Err(RuntimeError::CorruptImage {
                    reference: reference.to_string(),
                    reason: format!("layer file `{}` is missing", path.display()),
                });
            }
            layers.push(path);
        }
        Ok(Image {
            reference: reference.to_string(),
            dir: dir.to_path_buf(),
            manifest,
            layers,
        })
    }

    fn ensure_images_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.images_dir).map_err(|err| RuntimeError::Io {
            path: self.images_dir.display().to_string(),
            message: err.to_string(),
        })
    }

    fn stage_dir(&self) -> Result<PathBuf> {
        self.ensure_images_dir()?;
        let path = self
            .images_dir
            .join(format!("{STAGED_PREFIX}{}", short_suffix()));
        fs::create_dir_all(&path).map_err(|err| RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        Ok(path)
    }
}

fn summarize(reference: &str, manifest: &ImageManifest) -> ImageSummary {
    ImageSummary {
        reference: reference.to_string(),
        database: manifest.database.clone(),
        version: manifest.version.clone(),
        architecture: manifest.architecture,
        digest: manifest.digest.clone(),
        layer_count: manifest.layers.len(),
    }
}

fn write_bundle(image: &Image, dest: &Path) -> Result<()> {
    let file = fs::File::create(dest).map_err(|err| RuntimeError::Io {
        path: dest.display().to_string(),
        message: err.to_string(),
    })?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder
        .append_path_with_name(image.dir.join(MANIFEST_FILE), MANIFEST_FILE)
        .map_err(|err| RuntimeError::Io {
            path: image.dir.join(MANIFEST_FILE).display().to_string(),
            message: err.to_string(),
        })?;
    for layer in &image.layers {
        let name = layer
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| RuntimeError::InvalidManifest {
                reason: format!("layer without file name at `{}`", layer.display()),
            })?;
        builder
            .append_path_with_name(layer, format!("{LAYERS_DIR}/{name}"))
            .map_err(|err| RuntimeError::Io {
                path: layer.display().to_string(),
                message: err.to_string(),
            })?;
    }
    let encoder = builder.into_inner().map_err(|err| RuntimeError::Io {
        path: dest.display().to_string(),
        message: err.to_string(),
    })?;
    let file = encoder.finish().map_err(|err| RuntimeError::Io {
        path: dest.display().to_string(),
        message: err.to_string(),
    })?;
    file.sync_all().map_err(|err| RuntimeError::Io {
        path: dest.display().to_string(),
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

pub fn write_atomic(path: &Path, payload: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_path = parent.join(format!(
        "{}.tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "index".into()),
        short_suffix()
    ));
    {
        let mut file = fs::File::create(&tmp_path).map_err(|err| RuntimeError::Io {
            path: tmp_path.display().to_string(),
            message: err.to_string(),
        })?;
        file.write_all(payload).map_err(|err| RuntimeError::Io {
            path: tmp_path.display().to_string(),
            message: err.to_string(),
        })?;
        file.sync_all().map_err(|err| RuntimeError::Io {
            path: tmp_path.display().to_string(),
            message: err.to_string(),
        })?;
    }
    fs::rename(&tmp_path, path).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })
}

pub fn verify_layer_file(path: &Path, layer: &LayerRef) -> Result<()> {
    verify_digest(path, &layer.digest).map_err(|err| match err {
        RuntimeError::DigestMismatch { expected, actual } => RuntimeError::DigestMismatch {
            expected: format!("{} ({})", expected, path.display()),
            actual,
        },
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::manifest::sample_manifest;
    use crate::image::manifest::{Healthcheck, ImageManifest};
    use crate::image::reference::join_reference;
    use std::io::Read;

    fn write_file(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

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
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn build_manifest(layer_bytes: &[u8]) -> ImageManifest {
        let mut manifest = sample_manifest();
        manifest.digest = String::new();
        let hex = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(layer_bytes))
        };
        manifest.layers = vec![LayerRef {
            digest: format!("sha256:{hex}"),
            size: layer_bytes.len() as u64,
        }];
        manifest.digest = manifest.compute_digest().unwrap();
        manifest
    }

    fn write_staged(root: &Path, manifest: &ImageManifest, layer_bytes: &[u8]) {
        let layers_dir = root.join("layers");
        let layer_path = manifest
            .layer_path(&layers_dir, &manifest.layers[0])
            .unwrap();
        write_file(&layer_path, layer_bytes);
        write_file(
            &root.join("manifest.json"),
            serde_json::to_vec_pretty(manifest).unwrap().as_ref(),
        );
    }

    fn write_bundle_file(root: &Path, manifest: &ImageManifest, layer_bytes: &[u8]) -> PathBuf {
        let staged_root = root.join("bundle-src");
        let layers_dir = staged_root.join("layers");
        let layer_path = manifest
            .layer_path(&layers_dir, &manifest.layers[0])
            .unwrap();
        let layer_name = layer_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        write_file(&layer_path, layer_bytes);
        write_file(
            &staged_root.join("manifest.json"),
            serde_json::to_vec_pretty(manifest).unwrap().as_ref(),
        );

        let bundle = root.join("bundle.tar.gz");
        let file = fs::File::create(&bundle).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = tar::Builder::new(encoder);
        builder
            .append_path_with_name(staged_root.join("manifest.json"), "manifest.json")
            .unwrap();
        builder
            .append_path_with_name(&layer_path, format!("layers/{layer_name}"))
            .unwrap();
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
        bundle
    }

    #[test]
    fn import_bundle_roundtrip() {
        let work = tempfile::tempdir().unwrap();
        let layer_bytes = layer_tar_gz(b"hello db");
        let manifest = build_manifest(&layer_bytes);
        let bundle = write_bundle_file(work.path(), &manifest, &layer_bytes);

        let store = ImageStore::new(work.path().join("images"));
        let image = store.import_bundle(&bundle).unwrap();
        assert_eq!(image.reference, join_reference("mariadb", "11.4"));
        assert_eq!(image.manifest.layers.len(), 1);
        assert!(image.layers[0].is_file());

        store.verify("mariadb:11.4").unwrap();

        let from_disk = store.get("mariadb:11.4").unwrap();
        assert_eq!(from_disk.manifest.digest, manifest.digest);
        let mut layer_content = Vec::new();
        fs::File::open(&from_disk.layers[0])
            .unwrap()
            .read_to_end(&mut layer_content)
            .unwrap();
        assert_eq!(layer_content, layer_bytes);
    }

    #[test]
    fn import_bundle_rejects_tampered_manifest() {
        let work = tempfile::tempdir().unwrap();
        let layer_bytes = layer_tar_gz(b"hello db");
        let mut manifest = build_manifest(&layer_bytes);
        manifest.startup_command = vec!["evil".into()];
        let bundle = write_bundle_file(work.path(), &manifest, &layer_bytes);

        let store = ImageStore::new(work.path().join("images"));
        let err = store.import_bundle(&bundle).unwrap_err();
        assert!(matches!(err, RuntimeError::DigestMismatch { .. }));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn import_bundle_rejects_missing_layer_file() {
        let work = tempfile::tempdir().unwrap();
        let layer_bytes = layer_tar_gz(b"hello db");
        let manifest = build_manifest(&layer_bytes);
        let extra = LayerRef {
            digest: format!("sha256:{}", "cd".repeat(32)),
            size: 3,
        };
        let mut manifest = manifest;
        manifest.layers.push(extra);
        manifest.digest = manifest.compute_digest().unwrap();
        let bundle = write_bundle_file(work.path(), &manifest, &layer_bytes);

        let store = ImageStore::new(work.path().join("images"));
        let err = store.import_bundle(&bundle).unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidManifest { .. }));
    }

    #[test]
    fn import_bundle_rejects_layer_digest_mismatch() {
        let work = tempfile::tempdir().unwrap();
        let layer_bytes = layer_tar_gz(b"hello db");
        let mut manifest = build_manifest(&layer_bytes);
        manifest.layers[0].digest = format!("sha256:{}", "ef".repeat(32));
        manifest.digest = manifest.compute_digest().unwrap();
        let bundle = write_bundle_file(work.path(), &manifest, &layer_bytes);

        let store = ImageStore::new(work.path().join("images"));
        let err = store.import_bundle(&bundle).unwrap_err();
        assert!(matches!(err, RuntimeError::DigestMismatch { .. }));
    }

    #[test]
    fn import_bundle_rejects_duplicate_image() {
        let work = tempfile::tempdir().unwrap();
        let layer_bytes = layer_tar_gz(b"hello db");
        let manifest = build_manifest(&layer_bytes);
        let bundle = write_bundle_file(work.path(), &manifest, &layer_bytes);

        let store = ImageStore::new(work.path().join("images"));
        store.import_bundle(&bundle).unwrap();
        let err = store.import_bundle(&bundle).unwrap_err();
        assert!(
            matches!(err, RuntimeError::ImageAlreadyExists { ref reference } if reference == "mariadb:11.4")
        );
    }

    #[test]
    fn import_staged_from_reference_dir() {
        let work = tempfile::tempdir().unwrap();
        let layer_bytes = layer_tar_gz(b"staged");
        let manifest = build_manifest(&layer_bytes);
        let staged = work.path().join("staged");
        write_staged(&staged, &manifest, &layer_bytes);

        let store = ImageStore::new(work.path().join("images"));
        let image = store.import_staged(&staged).unwrap();
        assert_eq!(image.reference, "mariadb:11.4");
        assert!(!staged.exists());
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn export_bundle_roundtrip_into_new_store() {
        let work = tempfile::tempdir().unwrap();
        let layer_bytes = layer_tar_gz(b"export me");
        let manifest = build_manifest(&layer_bytes);
        let bundle = write_bundle_file(work.path(), &manifest, &layer_bytes);

        let store = ImageStore::new(work.path().join("images"));
        store.import_bundle(&bundle).unwrap();

        let export_path = work.path().join("out").join("mini.tar.gz");
        store.export_bundle("mariadb:11.4", &export_path).unwrap();
        assert!(export_path.is_file());

        let second = ImageStore::new(work.path().join("images2"));
        let image = second.import_bundle(&export_path).unwrap();
        second.verify(&image.reference).unwrap();
        assert_eq!(image.manifest.database, "mariadb");
    }

    #[test]
    fn export_missing_image_fails_typed() {
        let work = tempfile::tempdir().unwrap();
        let store = ImageStore::new(work.path().join("images"));
        let err = store
            .export_bundle("mariadb:11.4", &work.path().join("x.tar.gz"))
            .unwrap_err();
        assert!(matches!(err, RuntimeError::ImageNotFound { .. }));
    }

    #[test]
    fn remove_deletes_image_and_reports_missing() {
        let work = tempfile::tempdir().unwrap();
        let layer_bytes = layer_tar_gz(b"bye");
        let manifest = build_manifest(&layer_bytes);
        let bundle = write_bundle_file(work.path(), &manifest, &layer_bytes);

        let store = ImageStore::new(work.path().join("images"));
        store.import_bundle(&bundle).unwrap();
        store.remove("mariadb:11.4").unwrap();
        assert!(store.list().unwrap().is_empty());
        let err = store.get("mariadb:11.4").unwrap_err();
        assert!(matches!(err, RuntimeError::ImageNotFound { .. }));
    }

    #[test]
    fn list_returns_sorted_summaries() {
        let work = tempfile::tempdir().unwrap();
        let store = ImageStore::new(work.path().join("images"));
        for (database, version) in [("mysql", "9.0"), ("mariadb", "11.4"), ("postgres", "17")] {
            let mut manifest = sample_manifest();
            manifest.database = database.into();
            manifest.version = version.into();
            manifest.digest = String::new();
            let layer_bytes = layer_tar_gz(format!("{database} {version}").as_bytes());
            let hex = {
                use sha2::{Digest, Sha256};
                format!("{:x}", Sha256::digest(&layer_bytes))
            };
            manifest.layers = vec![LayerRef {
                digest: format!("sha256:{hex}"),
                size: layer_bytes.len() as u64,
            }];
            manifest.digest = manifest.compute_digest().unwrap();
            let staged = work.path().join(format!("staged-{database}"));
            write_staged(&staged, &manifest, &layer_bytes);
            store.import_staged(&staged).unwrap();
        }
        let references: Vec<String> = store
            .list()
            .unwrap()
            .into_iter()
            .map(|s| s.reference)
            .collect();
        assert_eq!(references, vec!["mariadb:11.4", "mysql:9.0", "postgres:17"]);
    }

    #[test]
    fn list_ignores_staging_directories() {
        let work = tempfile::tempdir().unwrap();
        let store = ImageStore::new(work.path().join("images"));
        store.stage_dir().unwrap();
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn verify_fails_typed_on_corrupt_layer() {
        let work = tempfile::tempdir().unwrap();
        let layer_bytes = layer_tar_gz(b"pristine");
        let manifest = build_manifest(&layer_bytes);
        let bundle = write_bundle_file(work.path(), &manifest, &layer_bytes);
        let store = ImageStore::new(work.path().join("images"));
        let image = store.import_bundle(&bundle).unwrap();
        fs::write(&image.layers[0], b"corrupted").unwrap();
        let err = store.verify("mariadb:11.4").unwrap_err();
        assert!(matches!(err, RuntimeError::CorruptImage { .. }));
    }

    #[test]
    fn healthcheck_serde_roundtrip() {
        let healthcheck = Healthcheck {
            port: 3306,
            timeout_secs: 5,
        };
        let value = serde_json::to_value(healthcheck).unwrap();
        let decoded: Healthcheck = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, healthcheck);
    }
}
