use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tar::Builder;

use crate::error::{Result, RuntimeError};
use crate::storage::volume::Volume;

const LOCK_DIR: &str = ".locks";
const LOCK_ATTEMPTS: u32 = 20;
const LOCK_RETRY: Duration = Duration::from_millis(100);
const VOLUME_MARKER: &str = ".tokedb-volume";

pub struct VolumeLock {
    path: PathBuf,
    held: bool,
}

impl VolumeLock {
    pub fn acquire(volumes_dir: &Path, name: &str) -> Result<VolumeLock> {
        let lock_dir = volumes_dir.join(LOCK_DIR);
        fs::create_dir_all(&lock_dir).map_err(|err| RuntimeError::Io {
            path: lock_dir.display().to_string(),
            message: err.to_string(),
        })?;
        let path = lock_dir.join(format!("{name}.lock"));
        for _ in 0..LOCK_ATTEMPTS {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => {
                    return Ok(VolumeLock { path, held: true });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(LOCK_RETRY);
                }
                Err(err) => {
                    return Err(RuntimeError::Io {
                        path: path.display().to_string(),
                        message: err.to_string(),
                    });
                }
            }
        }
        Err(RuntimeError::VolumeBusy {
            name: name.to_string(),
        })
    }
}

impl Drop for VolumeLock {
    fn drop(&mut self) {
        if self.held {
            self.held = false;
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn backup_volume(volume: &Volume, dest_dir: &Path) -> Result<PathBuf> {
    let name = volume.name.clone();
    let volumes_dir = volume
        .path
        .parent()
        .ok_or_else(|| RuntimeError::InvalidConfig("volume has no parent".to_string()))?;

    fs::create_dir_all(dest_dir).map_err(|err| RuntimeError::Io {
        path: dest_dir.display().to_string(),
        message: err.to_string(),
    })?;

    let _lock = VolumeLock::acquire(volumes_dir, &name)?;

    let backup_path = dest_dir.join(format!("{name}.tar"));
    let file = File::create(&backup_path).map_err(|err| RuntimeError::Io {
        path: backup_path.display().to_string(),
        message: err.to_string(),
    })?;
    let mut writer = BufWriter::new(file);
    {
        let mut builder = Builder::new(&mut writer);
        builder
            .append_dir_all(&name, &volume.path)
            .map_err(|err| RuntimeError::Layer(format!("backup tar: {err}")))?;
        builder
            .finish()
            .map_err(|err| RuntimeError::Layer(format!("backup tar: {err}")))?;
    }
    writer.flush().map_err(|err| RuntimeError::Io {
        path: backup_path.display().to_string(),
        message: err.to_string(),
    })?;
    Ok(backup_path)
}

pub fn backup_archive_prefix(volume_name: &str) -> String {
    format!("{volume_name}/")
}

pub fn is_volume_archive_entry(volume_name: &str, entry_path: &str) -> bool {
    let prefix = backup_archive_prefix(volume_name);
    entry_path.starts_with(&prefix) && !entry_path.ends_with('/')
}

pub fn volume_data_entry(volume_name: &str, entry_path: &str) -> Option<String> {
    let prefix = backup_archive_prefix(volume_name);
    if is_volume_archive_entry(volume_name, entry_path) {
        let rel = entry_path
            .trim_start_matches(&prefix)
            .trim_start_matches('/');
        if rel.is_empty() || rel == VOLUME_MARKER {
            None
        } else {
            Some(rel.to_string())
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_released_on_drop() {
        let work = tempfile::tempdir().unwrap();
        let volumes = work.path().join("volumes");
        fs::create_dir_all(&volumes).unwrap();
        {
            let lock = VolumeLock::acquire(&volumes, "data").unwrap();
            assert!(matches!(
                VolumeLock::acquire(&volumes, "data"),
                Err(RuntimeError::VolumeBusy { ref name }) if name == "data"
            ));
            drop(lock);
        }
        assert!(VolumeLock::acquire(&volumes, "data").is_ok());
    }

    #[test]
    fn backup_creates_consistent_tar() {
        let work = tempfile::tempdir().unwrap();
        let store = crate::storage::volume::VolumeStore::new(work.path().join("volumes"));
        let volume = store.create("data").unwrap();
        fs::create_dir_all(volume.path.join("db").join("x")).unwrap();
        fs::write(volume.path.join("db").join("x").join("rows.bin"), b"12345").unwrap();

        let backups = work.path().join("backups");
        let archive = backup_volume(&volume, &backups).unwrap();
        assert!(archive.is_file());
        assert_eq!(archive, backups.join("data.tar"));

        let mut entries = Vec::new();
        let archive_file = File::open(&archive).unwrap();
        let mut archive_reader = tar::Archive::new(archive_file);
        for entry in archive_reader.entries().unwrap() {
            let entry = entry.unwrap();
            entries.push(
                entry
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .trim_end_matches('/')
                    .to_string(),
            );
        }
        entries.sort();
        let expect = [
            "data",
            "data/.tokedb-volume",
            "data/db",
            "data/db/x",
            "data/db/x/rows.bin",
        ];
        assert_eq!(entries, expect);
    }

    #[test]
    fn backup_respects_volume_lock() {
        let work = tempfile::tempdir().unwrap();
        let store = crate::storage::volume::VolumeStore::new(work.path().join("volumes"));
        let volume = store.create("data").unwrap();
        let backups = work.path().join("backups");

        let _lock = VolumeLock::acquire(&work.path().join("volumes"), "data").unwrap();
        assert!(matches!(
            backup_volume(&volume, &backups),
            Err(RuntimeError::VolumeBusy { ref name }) if name == "data"
        ));
    }

    #[test]
    fn archive_entry_helpers() {
        assert!(is_volume_archive_entry("data", "data/db/rows.bin"));
        assert!(is_volume_archive_entry("data", "data/.tokedb-volume"));
        assert!(!is_volume_archive_entry("data", "data/"));
        assert!(!is_volume_archive_entry("data", "other/db"));
        assert_eq!(
            volume_data_entry("data", "data/db/rows.bin"),
            Some("db/rows.bin".to_string())
        );
        assert_eq!(volume_data_entry("data", "data/"), None);
        assert_eq!(volume_data_entry("data", "data/.tokedb-volume"), None);
        assert_eq!(volume_data_entry("data", "other/db"), None);
    }
}
