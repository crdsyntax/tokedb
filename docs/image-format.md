# tokedb — Image Format

Source modules: `src/image/`, `src/database/`.

## 1. References

References are `name[:tag]`:

- `name` — lowercase, digits, `.`, `_`, `-`; no leading/trailing `.`/`-`;
  non-empty.
- `tag` — alphanumeric, `.`, `_`, `-`; no leading `.`/`-`; non-empty; defaults
  to `latest`.

`parse_reference` returns `(name, tag)` and rejects empty/separator-laden
input with `RuntimeError::InvalidReference`; `join_reference` rebuilds the
string. On disk, references map to **two directory segments**
(`images/<name>/<tag>/`) because `:` is not valid in NTFS filenames.

## 2. Manifest schema

```rust
pub struct ImageManifest {
    pub database: String,          // "mariadb", "mysql", "postgres", "mongodb"
    pub version: String,           // e.g. "11.4"
    pub architecture: Architecture,// "amd64" | "arm64"
    pub digest: String,            // self-referential sha256:<64 hex>
    pub default_port: u16,
    pub data_directory: String,    // e.g. "/var/lib/mysql"
    pub healthcheck: Healthcheck,  // { port, timeout_secs }
    pub startup_command: Vec<String>,
    pub layers: Vec<LayerRef>,     // { digest: "sha256:<hex>", size: u64 }
}
```

`validate()` enforces: valid name/tag, digest format, non-zero
`default_port`, non-empty `data_directory` (no trailing `/`), non-zero
healthcheck port/timeout, non-empty `startup_command`, ≥ 1 layer, valid layer
digests/sizes, no duplicate layer digests.

## 3. Canonical digest

The manifest digest is **self-referential**:

1. Serialize the manifest to a `serde_json::Value`.
2. Remove the `digest` field.
3. Re-serialize to compact JSON (serde_json maps are `BTreeMap`, so keys come
   out **sorted** — a canonical form reproducible in any JSON library with
   `sort_keys=True`).
4. `sha256` of those bytes → `sha256:<64 hex>`.

`verify_digest()` recomputes and compares; mismatch →
`RuntimeError::DigestMismatch`. Layer digests are the SHA-256 of the
**compressed** `.tar.gz` blob (`digest_of_bytes` / `sha256_file`).

## 4. On-disk layout (ImageStore)

With data root `R`:

```
R/images/<database>/<version>/
├── manifest.json
└── layers/
    └── <sha256-hex>.tar.gz
```

- `import_bundle(path)` — unzips a bundle tar.gz (staging dir `.tmp-<uuid8>`
  under `images/`), then delegates to `import_staged`.
- `import_staged(staged)` is the **single verification point**: manifest parse
  + validate + digest + `verify_layers_in_dir` (every declared layer present
  with matching digest and size, no extra files), rejects existing images
  (`ImageAlreadyExists`), then `fs::rename` into place.
- `export_bundle(reference, dest)` — verifies, writes a gzip tar (temp + rename)
  containing `manifest.json` and `layers/<hex>.tar.gz`.
- `get` / `verify` (wraps errors into `CorruptImage`) / `remove` / `list`
  (walks `images/<name>/<tag>/`, skips staging dirs, sorted by reference).
- Shared helpers `write_atomic`, `ensure_dir`, `verify_layer_file` are also
  used by the registries.

## 5. Bundles (import/export)

A bundle is a gzipped tar with exactly:

```
manifest.json
layers/<sha256-hex>.tar.gz
```

## 6. Registries

`Registry` trait — the fetch does **not** verify; it materializes a staging
directory that `ImageStore::import_staged` later validates. This keeps local
and remote semantics identical.

```rust
pub trait Registry {
    fn fetch(&self, reference: &str, staged_dir: &Path) -> Result<()>;
}
```

### Local registry (`registry/local.rs`)

```
<root>/
├── index.json              # { images: [{ reference, manifest_digest, layers }] }
└── blobs/
    ├── <hex>.json          # manifest blob (content-addressed)
    └── <hex>.tar.gz        # layer blob
```

- `publish(image)` copies manifest + layers into `blobs/`, upserts
  `index.json` (sorted by reference, atomic write).
- `has(reference)`, `remove(reference)` (missing → `ImageNotFound`), `fetch`.

### Remote registry (`registry/remote.rs`)

A subset of the Docker **Registry API v2** over HTTPS (reqwest blocking +
rustls, 120s timeout):

- `GET {base}/v2/{name}/manifests/{tag}` with
  `Accept: application/vnd.tokedb.image.manifest.v1+json` → deserialize,
  validate, verify digest, verify the reference matches the manifest.
- Per layer: `GET {base}/v2/{name}/blobs/{digest}` (full `sha256:...` digest in
  the URL) → content digest and size checked against the manifest
  (`DigestMismatch` / `InvalidManifest`).
- HTTP 404 → `ImageNotFound`; other non-2xx → `Registry(String)`.
- Writes `manifest.json` and `layers/<hex>.tar.gz` into the staging dir.

## 7. Engine specs (`src/database/`)

`DatabaseSpec` is a **struct + const table** (deliberately not a trait — the
specs are static and homogeneous, so dynamic dispatch would be needless):

```rust
pub struct DatabaseSpec {
    pub engine: &'static str,
    pub default_port: u16,
    pub data_directory: &'static str,
    pub healthcheck_port: u16,
    pub healthcheck_timeout_secs: u16,
    pub startup_command: &'static [&'static str],
    pub container_user: ContainerUser,
}
```

| engine | port | data_directory | startup_command | user |
|---|---|---|---|---|
| `mariadb` | 3306 | `/var/lib/mysql` | `mariadbd` | 999:999 |
| `mysql` | 3306 | `/var/lib/mysql` | `mysqld` | 999:999 |
| `postgres` | 5432 | `/var/lib/postgresql/data` | `postgres` | 999:999 |
| `mongodb` | 27017 | `/data/db` | `mongod` | 999:999 |

`all()` (ordered), `for_engine(engine)` (typed lookup), `engines()`. Every spec
runs as an **unprivileged user** (uid/gid 999) — no engine ever runs as root.