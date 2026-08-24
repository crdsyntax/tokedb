use std::path::Path;
use std::time::Duration;

use reqwest::blocking::{Client, Response};

use crate::error::{Result, RuntimeError};
use crate::image::layers::digest_of_bytes;
use crate::image::manifest::ImageManifest;
use crate::image::reference::parse_reference;
use crate::image::registry::{verify_manifest_reference, Registry};
use crate::image::storage::ensure_dir;

const MANIFEST_MEDIA_TYPE: &str = "application/vnd.tokedb.image.manifest.v1+json";

#[derive(Debug)]
pub struct RemoteRegistry {
    client: Client,
    base: String,
}

impl RemoteRegistry {
    pub fn new(base: impl Into<String>) -> Result<Self> {
        Self::with_client(
            base,
            Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .map_err(registry_error)?,
        )
    }

    pub fn with_client(base: impl Into<String>, client: Client) -> Result<Self> {
        let base = base.into();
        let trimmed = base.trim_end_matches('/');
        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
            return Err(RuntimeError::InvalidConfig(format!(
                "registry URL must start with http:// or https://: {base}"
            )));
        }
        Ok(RemoteRegistry {
            client,
            base: trimmed.to_string(),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }
}

impl Registry for RemoteRegistry {
    fn fetch(&self, reference: &str, staged_dir: &Path) -> Result<()> {
        let (name, tag) = parse_reference(reference)?;
        let manifest_url = format!("{}/v2/{name}/manifests/{tag}", self.base);
        let manifest_bytes = self.get_bytes(&manifest_url, reference)?;
        let manifest: ImageManifest = serde_json::from_slice(&manifest_bytes).map_err(|err| {
            RuntimeError::InvalidManifest {
                reason: err.to_string(),
            }
        })?;
        manifest.validate()?;
        manifest.verify_digest()?;
        verify_manifest_reference(&manifest, &name, &tag)?;

        ensure_dir(staged_dir)?;
        std::fs::write(staged_dir.join("manifest.json"), &manifest_bytes).map_err(|err| {
            RuntimeError::Io {
                path: staged_dir.join("manifest.json").display().to_string(),
                message: err.to_string(),
            }
        })?;
        let layers_dir = staged_dir.join("layers");
        ensure_dir(&layers_dir)?;
        for layer in &manifest.layers {
            let blob_url = format!("{}/v2/{name}/blobs/{}", self.base, layer.digest);
            let bytes = self.get_bytes(&blob_url, reference)?;
            let actual = digest_of_bytes(&bytes);
            if actual != layer.digest {
                return Err(RuntimeError::DigestMismatch {
                    expected: layer.digest.clone(),
                    actual,
                });
            }
            if bytes.len() as u64 != layer.size {
                return Err(RuntimeError::InvalidManifest {
                    reason: format!(
                        "layer `{}` size mismatch: expected {}, got {}",
                        layer.digest,
                        layer.size,
                        bytes.len()
                    ),
                });
            }
            let path = manifest.layer_path(&layers_dir, layer)?;
            std::fs::write(&path, &bytes).map_err(|err| RuntimeError::Io {
                path: path.display().to_string(),
                message: err.to_string(),
            })?;
        }
        Ok(())
    }
}

impl RemoteRegistry {
    fn get_bytes(&self, url: &str, reference: &str) -> Result<Vec<u8>> {
        let response: Response = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT, MANIFEST_MEDIA_TYPE)
            .send()
            .map_err(registry_error)?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(RuntimeError::ImageNotFound {
                reference: reference.to_string(),
            });
        }
        if !status.is_success() {
            return Err(RuntimeError::Registry(format!("GET {url}: HTTP {status}")));
        }
        response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(registry_error)
    }
}

fn registry_error(err: reqwest::Error) -> RuntimeError {
    RuntimeError::Registry(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::manifest::LayerRef;
    use crate::image::manifest::{read_manifest, sample_manifest};
    use crate::image::storage::ImageStore;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    const ABC_HEX: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

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

    fn serviceable_manifest(layer_bytes: &[u8]) -> (ImageManifest, Vec<u8>) {
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
        (manifest, layer_bytes.to_vec())
    }

    struct TestServer {
        base: String,
        stop: Arc<AtomicBool>,
        handle: std::thread::JoinHandle<()>,
    }

    type Handler = Arc<dyn Fn(&str) -> (u16, Vec<u8>) + Send + Sync>;

    impl TestServer {
        fn serve(handler: impl Fn(&str) -> (u16, Vec<u8>) + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let _ = listener.set_nonblocking(true);
            let stop = Arc::new(AtomicBool::new(false));
            let stop_flag = stop.clone();
            let handler: Handler = Arc::new(handler);
            let handle = std::thread::spawn(move || {
                while !stop_flag.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let handler = handler.clone();
                            std::thread::spawn(move || serve_connection(stream, &*handler));
                        }
                        Err(err) => {
                            if err.kind() != std::io::ErrorKind::WouldBlock
                                && err.kind() != std::io::ErrorKind::Interrupted
                            {
                                eprintln!("[test-server] accept error: {err}");
                            }
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                }
            });
            TestServer { base, stop, handle }
        }

        fn url(&self) -> String {
            self.base.clone()
        }

        fn shutdown(self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = self.handle.join();
        }
    }

    fn serve_connection(mut stream: TcpStream, handler: &dyn Fn(&str) -> (u16, Vec<u8>)) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut idle_waits = 0;
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => {
                    if buffer.is_empty() && idle_waits < 20 {
                        idle_waits += 1;
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    break;
                }
                Ok(n) => {
                    buffer.extend_from_slice(&chunk[..n]);
                    if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        if buffer.is_empty() {
            return;
        }
        let request_line = String::from_utf8_lossy(&buffer);
        let path = request_line.split_whitespace().nth(1).unwrap_or("/");
        let (status, body) = handler(path);
        let status_text = match status {
            200 => "OK",
            404 => "Not Found",
            _ => "Internal Server Error",
        };
        let header = format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&body);
    }

    fn client() -> Client {
        Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    }

    #[test]
    fn pull_happy_path_from_http_registry() {
        let (manifest, bytes) = serviceable_manifest(&layer_tar_gz(b"hello registry"));
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let blob_digest = manifest.layers[0].digest.clone();
        let server = TestServer::serve(move |path| {
            if path.starts_with("/v2/mariadb/manifests/11.4") {
                (200, manifest_bytes.clone())
            } else if path == format!("/v2/mariadb/blobs/{blob_digest}") {
                (200, bytes.clone())
            } else {
                (404, Vec::new())
            }
        });

        let registry = RemoteRegistry::with_client(server.url(), client()).unwrap();
        assert_eq!(registry.base_url(), server.url());
        let work = tempfile::tempdir().unwrap();
        let staged = work.path().join("pulled");
        registry.fetch("mariadb:11.4", &staged).unwrap();
        assert!(staged.join("manifest.json").is_file());

        let store = ImageStore::new(work.path().join("images"));
        let image = store.import_staged(&staged).unwrap();
        store.verify(&image.reference).unwrap();
        assert_eq!(image.manifest.database, "mariadb");
        server.shutdown();
    }

    #[test]
    fn pull_fails_typed_on_digest_mismatch() {
        let (manifest, _) = serviceable_manifest(&layer_tar_gz(b"good layer"));
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let server = TestServer::serve(move |path| {
            if path.starts_with("/v2/mariadb/manifests/11.4") {
                (200, manifest_bytes.clone())
            } else {
                (200, b"different bytes than the digest describes".to_vec())
            }
        });

        let registry = RemoteRegistry::with_client(server.url(), client()).unwrap();
        let work = tempfile::tempdir().unwrap();
        let staged = work.path().join("pulled");
        std::fs::create_dir_all(&staged).unwrap();
        let err = registry.fetch("mariadb:11.4", &staged).unwrap_err();
        assert!(
            matches!(err, RuntimeError::DigestMismatch { .. }),
            "{err:?}"
        );
        server.shutdown();
    }

    #[test]
    fn pull_404_manifest_is_image_not_found() {
        let server = TestServer::serve(|_| (404, Vec::new()));
        let registry = RemoteRegistry::with_client(server.url(), client()).unwrap();
        let work = tempfile::tempdir().unwrap();
        let staged = work.path().join("pulled");
        std::fs::create_dir_all(&staged).unwrap();
        let err = registry.fetch("mariadb:11.4", &staged).unwrap_err();
        assert!(matches!(err, RuntimeError::ImageNotFound { .. }), "{err:?}");
        server.shutdown();
    }

    #[test]
    fn pull_rejects_reference_mismatch() {
        let (mut manifest, bytes) = serviceable_manifest(&layer_tar_gz(b"ref mismatch"));
        manifest.database = "mysql".into();
        manifest.digest = manifest.compute_digest().unwrap();
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let blob_digest = manifest.layers[0].digest.clone();
        let server = TestServer::serve(move |path| {
            if path.starts_with("/v2/mariadb/manifests/11.4") {
                (200, manifest_bytes.clone())
            } else if path == format!("/v2/mariadb/blobs/{blob_digest}") {
                (200, bytes.clone())
            } else {
                (404, Vec::new())
            }
        });

        let registry = RemoteRegistry::with_client(server.url(), client()).unwrap();
        let work = tempfile::tempdir().unwrap();
        let staged = work.path().join("pulled");
        std::fs::create_dir_all(&staged).unwrap();
        let err = registry.fetch("mariadb:11.4", &staged).unwrap_err();
        assert!(
            matches!(err, RuntimeError::InvalidManifest { .. }),
            "{err:?}"
        );
        server.shutdown();
    }

    #[test]
    fn pull_rejects_tampered_manifest_digest() {
        let (mut manifest, bytes) = serviceable_manifest(&layer_tar_gz(b"tampered"));
        manifest.startup_command = vec!["evil".into()];
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let blob_digest = manifest.layers[0].digest.clone();
        let server = TestServer::serve(move |path| {
            if path.starts_with("/v2/mariadb/manifests/11.4") {
                (200, manifest_bytes.clone())
            } else if path == format!("/v2/mariadb/blobs/{blob_digest}") {
                (200, bytes.clone())
            } else {
                (404, Vec::new())
            }
        });

        let registry = RemoteRegistry::with_client(server.url(), client()).unwrap();
        let work = tempfile::tempdir().unwrap();
        let staged = work.path().join("pulled");
        std::fs::create_dir_all(&staged).unwrap();
        let err = registry.fetch("mariadb:11.4", &staged).unwrap_err();
        assert!(
            matches!(err, RuntimeError::DigestMismatch { .. }),
            "{err:?}"
        );
        server.shutdown();
    }

    #[test]
    fn with_client_rejects_non_http_urls() {
        for bad in ["ftp://x/y", "file:///tmp", "not-a-url", "localhost:5000"] {
            let err = RemoteRegistry::with_client(bad, client()).unwrap_err();
            assert!(
                matches!(err, RuntimeError::InvalidConfig(_)),
                "{bad}: {err:?}"
            );
        }
    }

    #[test]
    fn server_helper_serves_exact_paths() {
        let server = TestServer::serve(|path| {
            if path == "/v2/mariadb/manifests/11.4" {
                (200, b"manifest-json".to_vec())
            } else {
                (200, b"blob-bytes".to_vec())
            }
        });
        let response =
            reqwest::blocking::get(format!("{}/v2/mariadb/manifests/11.4", server.url())).unwrap();
        assert_eq!(response.text().unwrap(), "manifest-json");
        let blob_url = format!("{}/v2/mariadb/blobs/{}", server.url(), ABC_HEX);
        let response = reqwest::blocking::get(blob_url).unwrap();
        assert_eq!(response.text().unwrap(), "blob-bytes");
        server.shutdown();
    }

    #[test]
    fn read_manifest_roundtrip_helper() {
        let (manifest, _) = serviceable_manifest(&layer_tar_gz(b"read"));
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("manifest.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let decoded = read_manifest(&path).unwrap();
        assert_eq!(decoded.database, "mariadb");
    }
}
