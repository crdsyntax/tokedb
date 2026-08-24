use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

use crate::error::{Result, RuntimeError};

pub fn unpack_layer(source: &Path, dest: &Path) -> Result<u64> {
    let file = File::open(source).map_err(|err| RuntimeError::Io {
        path: source.display().to_string(),
        message: err.to_string(),
    })?;
    let mut buffered = BufReader::new(file);
    let gzip = is_gzip(&mut buffered)?;

    let reader: Box<dyn Read> = if gzip {
        Box::new(GzDecoder::new(buffered))
    } else {
        Box::new(buffered)
    };

    let mut archive = tar::Archive::new(reader);
    let mut extracted = 0u64;
    let entries = archive
        .entries()
        .map_err(|err| RuntimeError::Layer(err.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|err| RuntimeError::Layer(err.to_string()))?;
        let path = entry
            .path()
            .map_err(|err| RuntimeError::Layer(err.to_string()))?;
        validate_entry_path(&path)?;
        entry.unpack_in(dest).map_err(|err| RuntimeError::Io {
            path: dest.display().to_string(),
            message: err.to_string(),
        })?;
        extracted += 1;
    }
    Ok(extracted)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn validate_entry_path(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(RuntimeError::UnsafeLayer {
            path: path.display().to_string(),
        });
    }
    Ok(())
}

fn is_gzip(reader: &mut BufReader<File>) -> Result<bool> {
    let mut magic = [0u8; 2];
    reader
        .read_exact(&mut magic)
        .map_err(|err| RuntimeError::Io {
            path: String::new(),
            message: err.to_string(),
        })?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|err| RuntimeError::Io {
            path: String::new(),
            message: err.to_string(),
        })?;
    Ok(magic == [0x1f, 0x8b])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    const SHA256_OF_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn ustar_entry(name: &str, content: &[u8]) -> Vec<u8> {
        let mut header = [0u8; 512];
        let name_bytes = name.as_bytes();
        header[..name_bytes.len()].copy_from_slice(name_bytes);
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        let size = format!("{:011o}", content.len());
        header[124..135].copy_from_slice(size.as_bytes());
        header[135..136].copy_from_slice(b"\0");
        header[156..157].copy_from_slice(b"0");
        header[257..262].copy_from_slice(b"ustar");
        for byte in header[148..156].iter_mut() {
            *byte = b' ';
        }
        let sum: u32 = header.iter().map(|byte| *byte as u32).sum();
        let checksum = format!("{:06o}\0 ", sum);
        header[148..156].copy_from_slice(checksum.as_bytes());

        let mut out = header.to_vec();
        out.extend_from_slice(content);
        let padding = (512 - content.len() % 512) % 512;
        out.extend(std::iter::repeat(0u8).take(padding));
        out.extend_from_slice(&[0u8; 1024]);
        out
    }

    #[test]
    fn validate_entry_path_accepts_relative_paths() {
        for path in ["file.txt", "a/b/c.txt", "./file.txt"] {
            assert!(validate_entry_path(Path::new(path)).is_ok(), "{path}");
        }
    }

    #[test]
    fn validate_entry_path_rejects_unsafe_paths() {
        let absolute = if cfg!(windows) {
            "C:\\absolute"
        } else {
            "/absolute"
        };
        for path in ["../evil", "a/../b", "a/../../b", absolute] {
            let err = validate_entry_path(Path::new(path)).unwrap_err();
            assert!(matches!(err, RuntimeError::UnsafeLayer { .. }), "{path}");
        }
    }

    #[test]
    fn unpack_layer_extracts_plain_tar() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("layer.tar");
        let dest = temp.path().join("rootfs");
        fs::create_dir_all(&dest).unwrap();

        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "hello.txt", "hello".as_bytes())
                .unwrap();
            builder.finish().unwrap();
        }
        fs::write(&source, tar_bytes).unwrap();

        let count = unpack_layer(&source, &dest).unwrap();
        assert_eq!(count, 1);
        assert_eq!(fs::read_to_string(dest.join("hello.txt")).unwrap(), "hello");
    }

    #[test]
    fn unpack_layer_extracts_gzip_tar() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("layer.tar.gz");
        let dest = temp.path().join("rootfs");
        fs::create_dir_all(&dest).unwrap();

        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_size(3);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "a.txt", "abc".as_bytes())
                .unwrap();
            builder.finish().unwrap();
        }
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        fs::write(&source, encoder.finish().unwrap()).unwrap();

        let count = unpack_layer(&source, &dest).unwrap();
        assert_eq!(count, 1);
        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "abc");
    }

    #[test]
    fn unpack_layer_rejects_path_traversal_entry() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("evil.tar");
        let dest = temp.path().join("rootfs");
        fs::create_dir_all(&dest).unwrap();
        fs::write(&source, ustar_entry("../evil.txt", b"pwned")).unwrap();

        let err = unpack_layer(&source, &dest).unwrap_err();
        assert!(
            matches!(err, RuntimeError::UnsafeLayer { .. }),
            "unexpected: {err:?}"
        );
        assert!(!temp.path().join("evil.txt").exists());
    }

    #[test]
    fn sha256_file_matches_known_digest() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("data.bin");
        fs::write(&file, b"abc").unwrap();
        assert_eq!(sha256_file(&file).unwrap(), SHA256_OF_ABC);
    }
}
