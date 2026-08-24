use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, RuntimeError};
use crate::filesystem::sha256_file;
use crate::image::layers::{is_valid_digest, layer_hex};
use crate::image::reference::{valid_name, valid_tag};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Architecture {
    Amd64,
    Arm64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Healthcheck {
    pub port: u16,
    pub timeout_secs: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerRef {
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageManifest {
    pub database: String,
    pub version: String,
    pub architecture: Architecture,
    pub digest: String,
    pub default_port: u16,
    pub data_directory: String,
    pub healthcheck: Healthcheck,
    pub startup_command: Vec<String>,
    pub layers: Vec<LayerRef>,
}

impl ImageManifest {
    pub fn validate(&self) -> Result<()> {
        if !valid_name(&self.database) {
            return Err(invalid_manifest(format!(
                "invalid database name `{}`",
                self.database
            )));
        }
        if !valid_tag(&self.version) {
            return Err(invalid_manifest(format!(
                "invalid version tag `{}`",
                self.version
            )));
        }
        if !is_valid_digest(&self.digest) {
            return Err(invalid_manifest(format!(
                "invalid digest `{}`",
                self.digest
            )));
        }
        if self.default_port == 0 {
            return Err(invalid_manifest("default_port must not be zero"));
        }
        if self.data_directory.trim().is_empty() {
            return Err(invalid_manifest("data_directory must not be empty"));
        }
        if self.data_directory.ends_with('/') {
            return Err(invalid_manifest("data_directory must not end with '/'"));
        }
        if self.healthcheck.port == 0 {
            return Err(invalid_manifest("healthcheck port must not be zero"));
        }
        if self.healthcheck.timeout_secs == 0 {
            return Err(invalid_manifest("healthcheck timeout must not be zero"));
        }
        if self.startup_command.is_empty() {
            return Err(invalid_manifest("startup_command must not be empty"));
        }
        if self.layers.is_empty() {
            return Err(invalid_manifest("image must declare at least one layer"));
        }
        let mut seen = std::collections::HashSet::new();
        for layer in &self.layers {
            if !is_valid_digest(&layer.digest) {
                return Err(invalid_manifest(format!(
                    "invalid layer digest `{}`",
                    layer.digest
                )));
            }
            if layer.size == 0 {
                return Err(invalid_manifest(format!(
                    "layer `{}` has zero size",
                    layer.digest
                )));
            }
            if !seen.insert(layer.digest.clone()) {
                return Err(invalid_manifest(format!(
                    "duplicate layer digest `{}`",
                    layer.digest
                )));
            }
        }
        Ok(())
    }

    pub fn compute_digest(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).map_err(RuntimeError::from)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| invalid_manifest("manifest must be a JSON object"))?;
        object.remove("digest");
        let canonical = serde_json::to_string(&value).map_err(RuntimeError::from)?;
        Ok(format!("sha256:{:x}", Sha256::digest(canonical.as_bytes())))
    }

    pub fn verify_digest(&self) -> Result<()> {
        let actual = self.compute_digest()?;
        if actual != self.digest {
            return Err(RuntimeError::DigestMismatch {
                expected: self.digest.clone(),
                actual,
            });
        }
        Ok(())
    }

    pub fn layer_file_name(layer: &LayerRef) -> Result<String> {
        Ok(format!("{}.tar.gz", layer_hex(&layer.digest)?))
    }

    pub fn layer_path(&self, layers_dir: &Path, layer: &LayerRef) -> Result<PathBuf> {
        Ok(layers_dir.join(Self::layer_file_name(layer)?))
    }

    pub fn verify_layers_in_dir(&self, layers_dir: &Path) -> Result<()> {
        let entries = std::fs::read_dir(layers_dir).map_err(|err| RuntimeError::Io {
            path: layers_dir.display().to_string(),
            message: err.to_string(),
        })?;
        let mut expected_names = std::collections::HashSet::new();
        for layer in &self.layers {
            let name = Self::layer_file_name(layer)?;
            if !expected_names.insert(name.clone()) {
                return Err(invalid_manifest(format!("duplicate layer file `{name}`")));
            }
            let path = layers_dir.join(&name);
            if !path.is_file() {
                return Err(invalid_manifest(format!("layer file `{name}` is missing")));
            }
            let actual_hex = sha256_file(&path)?;
            let expected_hex = layer_hex(&layer.digest)?;
            if actual_hex != expected_hex {
                return Err(RuntimeError::DigestMismatch {
                    expected: layer.digest.clone(),
                    actual: format!("sha256:{actual_hex}"),
                });
            }
            let actual_size = std::fs::metadata(&path)
                .map_err(|err| RuntimeError::Io {
                    path: path.display().to_string(),
                    message: err.to_string(),
                })?
                .len();
            if actual_size != layer.size {
                return Err(invalid_manifest(format!(
                    "layer `{}` size mismatch: expected {}, got {}",
                    name, layer.size, actual_size
                )));
            }
        }
        for entry in entries {
            let entry = entry.map_err(|err| RuntimeError::Io {
                path: layers_dir.display().to_string(),
                message: err.to_string(),
            })?;
            let file_type = entry.file_type().map_err(|err| RuntimeError::Io {
                path: entry.path().display().to_string(),
                message: err.to_string(),
            })?;
            if file_type.is_file() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !expected_names.remove(&name) {
                    return Err(invalid_manifest(format!("unexpected layer file `{name}`")));
                }
            }
        }
        Ok(())
    }
}

pub fn read_manifest(path: &Path) -> Result<ImageManifest> {
    let bytes = std::fs::read(path).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    let manifest: ImageManifest =
        serde_json::from_slice(&bytes).map_err(|err| invalid_manifest(err.to_string()))?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn load_manifest(path: &Path, reference: &str) -> Result<ImageManifest> {
    read_manifest(path).map_err(|err| match err {
        RuntimeError::InvalidManifest { reason } => RuntimeError::CorruptImage {
            reference: reference.to_string(),
            reason,
        },
        other => other,
    })
}

#[cfg(test)]
pub(crate) fn sample_manifest() -> ImageManifest {
    ImageManifest {
        database: "mariadb".into(),
        version: "11.4".into(),
        architecture: Architecture::Amd64,
        digest: String::new(),
        default_port: 3306,
        data_directory: "/var/lib/mysql".into(),
        healthcheck: Healthcheck {
            port: 3306,
            timeout_secs: 5,
        },
        startup_command: vec!["mariadbd".into()],
        layers: vec![LayerRef {
            digest: format!("sha256:{}", "ab".repeat(32)),
            size: 3,
        }],
    }
}

fn invalid_manifest(reason: impl Into<String>) -> RuntimeError {
    RuntimeError::InvalidManifest {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::manifest::{Architecture, Healthcheck, ImageManifest, LayerRef};

    #[test]
    fn compute_digest_uses_sorted_key_canonical_form() {
        let manifest = ImageManifest {
            database: "testdb".into(),
            version: "1.0".into(),
            architecture: Architecture::Amd64,
            digest: String::new(),
            default_port: 3306,
            data_directory: "/var/lib/mysql".into(),
            healthcheck: Healthcheck {
                port: 3306,
                timeout_secs: 5,
            },
            startup_command: vec!["/bin/bash".into(), "-c".into(), "echo hi".into()],
            layers: vec![LayerRef {
                digest: "sha256:7865f25e9f2d9f015cd0fee3e2d8c3b9e4b3db4620b4eae9ed8ac9a2578005f8"
                    .into(),
                size: 103,
            }],
        };
        assert_eq!(
            manifest.compute_digest().unwrap(),
            "sha256:554a7c8dc25b65c5dbbe1601643dfc62e697cfb48b2e3b2ced1d8657b76de29e",
            "digest must match the sorted-key canonical form \
             (serde_json without preserve_order serializes maps in BTreeMap order)"
        );
    }

    #[test]
    fn manifest_serde_roundtrip() {
        let manifest = sample_manifest();
        let value = serde_json::to_value(&manifest).unwrap();
        let decoded: ImageManifest = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, manifest);
    }

    #[test]
    fn compute_digest_is_deterministic_and_sensitive() {
        let first = sample_manifest();
        let digest_a = first.compute_digest().unwrap();
        let digest_b = first.compute_digest().unwrap();
        assert_eq!(digest_a, digest_b);
        assert!(digest_a.starts_with("sha256:"));

        let mut changed = first;
        changed.version = "11.5".into();
        assert_ne!(changed.compute_digest().unwrap(), digest_a);
    }

    #[test]
    fn verify_digest_passes_for_computed_digest() {
        let mut manifest = sample_manifest();
        manifest.digest = manifest.compute_digest().unwrap();
        assert!(manifest.verify_digest().is_ok());
    }

    #[test]
    fn verify_digest_rejects_tampered_manifest() {
        let mut manifest = sample_manifest();
        manifest.digest = manifest.compute_digest().unwrap();
        manifest.startup_command = vec!["evil".into()];
        let err = manifest.verify_digest().unwrap_err();
        assert!(matches!(err, RuntimeError::DigestMismatch { .. }));
    }

    #[test]
    fn validate_rejects_invalid_fields() {
        let base = sample_manifest();
        let mut cases: Vec<Box<dyn FnOnce(ImageManifest) -> ImageManifest>> = Vec::new();
        cases.push(Box::new(|mut m| {
            m.database = "".into();
            m
        }));
        cases.push(Box::new(|mut m| {
            m.database = "BAD".into();
            m
        }));
        cases.push(Box::new(|mut m| {
            m.version = "-1".into();
            m
        }));
        cases.push(Box::new(|mut m| {
            m.digest = "md5:abc".into();
            m
        }));
        cases.push(Box::new(|mut m| {
            m.default_port = 0;
            m
        }));
        cases.push(Box::new(|mut m| {
            m.data_directory = " ".into();
            m
        }));
        cases.push(Box::new(|mut m| {
            m.data_directory = "/var/lib/mysql/".into();
            m
        }));
        cases.push(Box::new(|mut m| {
            m.healthcheck.port = 0;
            m
        }));
        cases.push(Box::new(|mut m| {
            m.healthcheck.timeout_secs = 0;
            m
        }));
        cases.push(Box::new(|mut m| {
            m.startup_command = Vec::new();
            m
        }));
        cases.push(Box::new(|mut m| {
            m.layers = Vec::new();
            m
        }));
        cases.push(Box::new(|mut m| {
            m.layers[0].size = 0;
            m
        }));
        cases.push(Box::new(|mut m| {
            m.layers[0].digest = "sha256:zz".into();
            m
        }));
        cases.push(Box::new(|mut m| {
            let duplicate = m.layers[0].clone();
            m.layers.push(duplicate);
            m
        }));
        for case in cases {
            let manifest = case(base.clone());
            let err = manifest.validate().unwrap_err();
            assert!(
                matches!(err, RuntimeError::InvalidManifest { .. }),
                "unexpected: {err:?}"
            );
        }
    }

    #[test]
    fn layer_file_name_uses_hex_only() {
        let layer = LayerRef {
            digest: format!("sha256:{}", "ab".repeat(32)),
            size: 3,
        };
        assert_eq!(
            ImageManifest::layer_file_name(&layer).unwrap(),
            format!("{}.tar.gz", "ab".repeat(32))
        );
    }

    #[test]
    fn verify_layers_in_dir_accepts_matching_layers() {
        let temp = tempfile::tempdir().unwrap();
        let layers_dir = temp.path().join("layers");
        std::fs::create_dir_all(&layers_dir).unwrap();
        let hex = sha256_file(&write_abc()).unwrap();
        std::fs::write(layers_dir.join(format!("{hex}.tar.gz")), b"abc").unwrap();
        let layer = LayerRef {
            digest: format!("sha256:{hex}"),
            size: 3,
        };
        let manifest = ImageManifest {
            layers: vec![layer],
            ..sample_manifest()
        };
        manifest.verify_layers_in_dir(&layers_dir).unwrap();
    }

    fn write_abc() -> PathBuf {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("abc.bin"), b"abc").unwrap();
        temp.keep().join("abc.bin")
    }
}
