use std::path::Path;

use crate::error::{Result, RuntimeError};
use crate::image::manifest::ImageManifest;

pub mod local;
pub mod remote;

pub use local::{LocalImageRef, LocalRegistry};
pub use remote::RemoteRegistry;

pub trait Registry {
    fn fetch(&self, reference: &str, staged_dir: &Path) -> Result<()>;
}

pub fn verify_manifest_reference(manifest: &ImageManifest, name: &str, tag: &str) -> Result<()> {
    if manifest.database != name || manifest.version != tag {
        return Err(RuntimeError::InvalidManifest {
            reason: format!(
                "reference mismatch: requested {name}:{tag}, manifest declares {}:{}",
                manifest.database, manifest.version
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::manifest::sample_manifest;
    use crate::image::reference::parse_reference;

    #[test]
    fn verify_manifest_reference_accepts_match() {
        let manifest = sample_manifest();
        verify_manifest_reference(&manifest, "mariadb", "11.4").unwrap();
    }

    #[test]
    fn verify_manifest_reference_rejects_mismatch() {
        let manifest = sample_manifest();
        let err = verify_manifest_reference(&manifest, "mysql", "11.4").unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidManifest { .. }));
    }

    #[test]
    fn unknown_engine_inference_is_strict() {
        let (name, _) = parse_reference("mariadb:11").unwrap();
        assert_eq!(name, "mariadb");
    }
}
