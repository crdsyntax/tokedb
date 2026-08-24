# tokedb — Architecture

## 1. Overview

`tokedb` is a **database-engine runtime**, not a generic container runtime. It
runs *only* database engine images (MariaDB, MySQL, PostgreSQL, MongoDB) — the
`database/` module is the closed set of supported engines, and the image format
is purpose-built for them (no Dockerfiles, no arbitrary apps/OS images). It
builds an isolated Linux process per database using kernel primitives directly —
namespaces, cgroups v2, overlayfs, pivot_root, capabilities and seccomp —
instead of shelling out to `docker` or `podman`. Ports are exposed through
userland proxies by default (no iptables/root for the default path).

The project is a Cargo workspace:

```
tokedb/
├── Cargo.toml            # workspace manifest
├── .cargo/config.toml    # `cargo dev` alias -> `cargo run --bin tokedb --`
└── tokedb-runtime/
    ├── Cargo.toml
    ├── src/              # library (src/lib.rs) + binary (src/main.rs)
    └── tests/            # Linux-gated integration tests
```

## 2. Crate layout

| Path | Purpose |
|---|---|
| `src/lib.rs` | Crate root; `#![deny(unsafe_code)]`; declares and re-exports the public API |
| `src/main.rs` | Binary `tokedb`; `#![forbid(unsafe_code)]`; tracing init + CLI dispatch |
| `src/cli/` | clap CLI (`Cli`, `Command`, per-subcommand args), dispatch logic, Windows WSL2 forwarding (`wsl.rs`) |
| `src/config.rs` | `RuntimeConfig`: data root and derived paths + bridge name |
| `src/state.rs` | `StateLayout`: typed path building + the single name-validation gate |
| `src/error.rs` | `RuntimeError` enum (26 typed variants), `Result<T>` alias |
| `src/platform/` | `Platform` trait: `LinuxPlatform` (cfg-gated) / `UnsupportedPlatform` |
| `src/runtime/` | Container model, store, lifecycle, process spawn, execution layer |
| `src/filesystem/` | Overlay mount, bind mounts, pivot_root, layer unpacking |
| `src/isolation/` | cgroups v2, capabilities, seccomp, privilege drop |
| `src/network/` | rtnetlink bridge/veth, userland port proxies, optional iptables |
| `src/storage/` | Volumes (persistent data) and lightweight backups |
| `src/image/` | Manifest, references, layers, `ImageStore`, registries (local + remote) |
| `src/database/` | `DatabaseSpec` static table for the four supported engines |

## 3. Public API (library consumers)

The library is embeddable. Consumers use:

- `RuntimeConfig` — configuration (paths + bridge name).
- `StateLayout` — typed on-disk path construction.
- `RuntimeError` / `Result<T>` — all fallible operations; errors are `Serialize`
  so they transport cleanly over IPC.
- `platform::current()` / `Platform` — capability probe.
- `runtime::*` — `ContainerStore`, `ContainerSpec`, `ContainerState`,
  `spawn_isolated` / `spawn_with_prep`, `CommandSpec`, `SpawnedProcess`.
- `image::*` — `ImageStore`, `ImageManifest`, registries.
- `storage::*` — `VolumeStore`, `Volume`, `backup_volume`.
- `database::*` — `DatabaseSpec`, `for_engine`, `engines`.

The binary (`src/main.rs`) is a thin wrapper: it only sets up tracing, parses
the CLI, and dispatches to `cli::run`.

## 4. Module dependency flow

```
CLI (src/main.rs) ──► cli ──► runtime::run ──► runtime::process ──► isolation / filesystem / network / image / storage
                          └──► image (import/export/pull) ──► image::registry
```

No circular module dependencies exist. The `cli` layer is the only place that
knows about the binary's command surface; the library exposes typed building
blocks beneath it.

## 5. Platform strategy

- A `Platform` trait with `name()` and `supports_containers()`.
- `ActivePlatform` resolves by `cfg`: `LinuxPlatform` on Linux, otherwise
  `UnsupportedPlatform`.
- All isolation code is gated with `cfg(target_os = "linux")`. Non-Linux builds
  compile the full library (so toketeo, which builds on Windows, can depend on
  it) but container operations return `RuntimeError::UnsupportedPlatform`.
- The Windows CLI is a thin client: it parses/validates arguments natively,
  then forwards the raw invocation to the Linux backend inside WSL2
  (`cli::wsl`). See [cli.md](./cli.md) for the WSL2 contract.
- Linux-only dependencies (`tokio`, `seccompiler`) live under
  `[target.'cfg(target_os = "linux")'.dependencies]` so non-Linux builds never
  compile them.

## 6. Safety discipline

- `#![deny(unsafe_code)]` at the crate root and `#![forbid(unsafe_code)]` in
  the binary.
- Only four Linux-only modules override with `#![allow(unsafe_code)]`, all for
  raw libc syscalls:
  - `filesystem/pivot.rs` — `pivot_root`, `umount2`.
  - `isolation/capabilities.rs` — `capset`, `PR_CAPBSET_DROP`.
  - `network/netlink.rs` — rtnetlink socket ABI.
  - `runtime/process.rs` (`linux_sys`) — `pipe2`, `fork`, `waitpid`, pid I/O.
- Unsafe blocks are scoped to the syscall surface; everything above them is
  safe Rust.

## 7. Cross-cutting conventions

- **Strong typing at every boundary** (CLI, IPC, filesystem): no dynamic maps.
  Persisted/CLI-visible schemas are `Serialize + Deserialize`.
- **One error type**: `RuntimeError` (thiserror) with typed variants; `From`
  impls for `std::io::Error`, `nix::errno::Errno` (Linux), `serde_json::Error`.
- **Validation at the edge**: names and references are validated before any
  path or store operation.
- **Atomic writes** everywhere state or blobs persist: temp file/dir + `rename`.
- **Digest scheme**: SHA-256 everywhere (manifest and layers), content-addressed
  layer blobs.
- **No panics on production paths**: all fallible operations return `Result`.
- **Structured logging** via `tracing`; credentials and payloads are never
  logged.
- **Code comments**: minimal; doc comments on public API surface where helpful,
  no superfluous inline comments.

## 8. Tests

- **Unit tests** run on any OS (`cargo test`): CLI parsing, config/state,
  image manifest/reference/store, registries, spec tables, serialization.
- **Linux integration tests** in `tests/` are gated by
  `#![cfg(all(target_os = "linux", feature = "integration-linux"))]` and require
  root (verified in WSL2):

| File | Covers |
|---|---|
| `pidns.rs` | process sees itself as PID 1 |
| `stop.rs` | SIGTERM stop semantics / signal traps |
| `rootfs.rs` | overlay rootfs + pivot; overlay validation |
| `cgroups.rs` | memory OOM kill; pids.max limits |
| `security.rs` | capability allowlist, seccomp EPERM, privilege drop |
| `volumes.rs` | volume persistence, bind into data dir, backup tar |
| `network.rs` | bridge connectivity, userland port proxies, gateway |
| `run_container.rs` | end-to-end start/stop/logs/volume/port flow |

Run with: `cargo test --features integration-linux` (needs root + overlayfs,
verified on WSL2).

## 9. Feature flags

- `integration-linux` — enables the Linux integration test binaries.
- `iptables` — enables `network::iptables` (direct DNAT as an alternative to
  the userland proxies; off by default).

## 10. Data layout

With data root `R`:

```
R/
├── images/            # <database>/<version>/manifest.json + layers/<hex>.tar.gz
├── containers/        # <id>/metadata.json + logs/{stdout,stderr}.log + rootfs/
├── volumes/           # <name>/ with .tokedb-volume marker; .locks/ for backups
└── registry/          # optional local registry: index.json + blobs/
```

## 11. Roadmap

F0–F8 complete (see `IMPLEMENTATION_PLAN.md` for the authoritative phase table
and verification notes). F9 — integration into toketeo — is pending:
`toketeo/src-tauri` runtime DTOs, `runtime_commands` (Tauri commands), and an
application-layer `runtime_service`, with the `tokedb` binary acting as the
privileged helper.