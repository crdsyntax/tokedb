# tokedb — Technical Documentation

`tokedb` is a container runtime in Rust specialized in running databases
(MariaDB, MySQL, PostgreSQL, MongoDB). It ships a standalone binary (`tokedb`)
and is also designed to be embedded as a library (`tokedb-runtime`) inside the
toketeo desktop application (Tauri 2), exposing operations over IPC commands.

## Document map

| Document | Contents |
|---|---|
| [architecture.md](./architecture.md) | Crate layout, module map, dependency flow, platform strategy, cross-cutting conventions |
| [container-runtime.md](./container-runtime.md) | Container model, lifecycle state machine, process spawn, filesystem, isolation, network, storage, execution flow |
| [image-format.md](./image-format.md) | Image manifest schema, references, digest scheme, on-disk layout, registries (local + remote) |
| [cli.md](./cli.md) | CLI subcommands, flags, environment variables, platform behavior |

## Quick reference

- **Workspace root**: `D:\Documents\GitHub\tokedb` (Cargo workspace, single
  member `tokedb-runtime`).
- **Binary**: `tokedb` (thin wrapper over the library).
- **Library crate**: `tokedb-runtime` (lib target `tokedb_runtime`).
- **Data root**: `TOKEDB_DATA_ROOT`, default `/var/lib/db-runtime` on Linux,
  `.db-runtime` elsewhere.
- **Platform support**: full container isolation is Linux-only (`cfg(target_os =
  "linux")`). On other platforms the library returns `UnsupportedPlatform`; on
  Windows the CLI acts as a thin client that forwards invocations to WSL2.

## Status

Implementation phases F0–F8 are complete and verified on both Windows (unit
tests) and WSL2 (unit + Linux integration tests). Phase F9 (integration into
toketeo) is pending. See the phase table in `IMPLEMENTATION_PLAN.md` for the
authoritative status.