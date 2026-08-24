# tokedb — CLI Reference

Source module: `src/cli/`. The binary is named `tokedb`; `cargo dev <args>`
runs it (`cargo run --bin tokedb -- <args>`).

```
tokedb <command> [args]

Container runtime for databases
```

All operations print typed errors to stderr as `error: <message>` and exit
with code 1.

## Commands

### Image management

| Command | Arguments | Behavior |
|---|---|---|
| `pull <reference>` | `--registry <URL_OR_PATH>` | Fetch an image from a registry and verify it into the store. No flag → default local registry at `<data_root>/registry`. `http(s)://` → remote Registry API v2. Other `://` scheme → `InvalidConfig`. Anything else → local registry path. |
| `import <path>` | — | Import a bundle tar.gz. |
| `export <reference> <output>` | — | Export a verified image as a bundle tar.gz. |
| `images` | — | List stored images: `reference  database:version  arch  digest  n layer(s)`. |
| `rmi <reference>` | — | Remove an image from the store. |

### Container management

| Command | Arguments | Behavior |
|---|---|---|
| `create <name> <image>` | `--memory-mb <u64>`, `--cpu-quota <f64>`, `--pids-max <u64>`, `--port <HOST:CONTAINER>` (repeatable) | Pull the image, parse port bindings, and create a container with an auto data volume `<name>-data` mounted at the image's `data_directory`. On volume creation failure the container is rolled back. |
| `start <name>` | — | Build rootfs, wire up network/cgroups, spawn the database process, stream logs, block until exit. |
| `stop <name>` | — | SIGTERM then SIGKILL after the grace period. |
| `logs <name>` | `-f, --follow` | Print captured `stdout.log` then `stderr.log`. |
| `inspect <name>` | — | Pretty-print the container `metadata.json`. |
| `destroy <name>` | — | Remove the container directory; the `<name>-data` volume is **kept**. |
| `list` | — | Table of `id  name  image  state  pid`. |

Port bindings accept `HOST:CONTAINER` or a bare `PORT` (same on both sides);
ports must be non-zero. Duplicate host ports are rejected.

## Configuration

| Variable | Purpose | Default |
|---|---|---|
| `TOKEDB_DATA_ROOT` | Runtime data root (images, containers, volumes) | Linux: `/var/lib/db-runtime`; else `.db-runtime` |
| `RUST_LOG` | tracing filter (via `tracing_subscriber::EnvFilter`) | `info` |

## Platform behavior

- **Linux**: full runtime. Container commands require root.
- **Windows**: the binary is a thin client — arguments are parsed/validated
  natively (so `--help`/errors work locally), then the raw invocation is
  forwarded into WSL2, where the real `tokedb` runs:

| Variable | Purpose | Default |
|---|---|---|
| `TOKEDB_WSL_DISTRO` | WSL distro to run in | `Ubuntu-24.04` |
| `TOKEDB_WSL_BINARY` | `tokedb` path inside the distro | `/usr/local/bin/tokedb` |
| `TOKEDB_DATA_ROOT` | data root exported into the distro | *(as above)* |

Windows paths (`C:\...`) are translated to `/mnt/...`; arguments containing
backslashes are resolved against the Windows cwd before translation; plain
names/references/URLs pass through.
- **Other platforms**: the library returns `RuntimeError::UnsupportedPlatform`
  for container operations.