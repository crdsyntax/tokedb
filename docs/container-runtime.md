# tokedb — Container Runtime

This document describes the container model, lifecycle, process spawn, and the
Linux isolation layers (filesystem, resources, security, network, storage), and
the execution flow that drives a container from `create` to `destroy`.

Source modules: `src/runtime/`, `src/filesystem/`, `src/isolation/`,
`src/network/`, `src/storage/`.

## 1. Container model

The persisted schema (the `containers/<id>/metadata.json` file) is the
`Container` struct:

```rust
pub struct Container {
    pub id: String,              // 8-char hex UUID
    pub name: String,            // unique, user-supplied
    pub image: String,           // image reference, e.g. "mariadb:11.4"
    pub command: CommandSpec,    // program + args + env + security + netns
    pub resources: ResourceLimits,   // memory_bytes / cpu_quota / pids_max
    pub volumes: Vec<VolumeMount>,   // { name, mount_path }
    pub ports: Vec<PortBinding>,     // { host_port, container_port, protocol }
    pub state: ContainerState,
    pub created_at: u64,         // unix seconds
    pub pid: Option<u32>,        // host PID while running
}
```

- `ResourceLimits` in `runtime::container` is structurally identical to
  `isolation::cgroups::ResourceLimits` (a deliberate mirror; the runtime model
  is decoupled from the cgroup implementation).
- `ContainerStore` persists via **atomic writes** (temp file + `sync_all` +
  `rename`) and refuses to persist a `Destroyed` state.

## 2. Lifecycle state machine

`src/runtime/lifecycle.rs` defines the allowed transitions; any other edge
returns `RuntimeError::InvalidTransition`.

```
Created → Starting → Running → Stopping → Stopped → Destroyed
             │           │                    │
             └───────────┴──── Stopped → Starting (restart)
```

- `Destroyed` is the only terminal state.
- Invalid transitions are typed errors, enforced in a single `transition()`
  function.

## 3. Process spawn: the fork + helper model

`src/runtime/process.rs` runs the container process as **PID 1 of a fresh PID
namespace**. Because `unshare(CLONE_NEWPID)` in the parent permanently breaks
`std::thread::spawn` in the whole process (the runtime needs threads for log
streaming, netns helpers, and tokio proxies), spawning uses a **fork + helper**
design:

```
runtime (parent)
  │  fork()
  ├── helper process
  │     ├── unshare(CLONE_NEWPID)        (_exit 125 on failure)
  │     ├── command.spawn() → container  (PID 1 of the new namespace; _exit 126 on failure)
  │     ├── write container pid → status pipe
  │     ├── close its stdio write-ends  (parent sees EOF)
  │     └── waitpid loop forwarding the raw wait status
  └── parent
        ├── reads pid from status pipe
        └── wraps read-ends into SpawnedProcess (stdout/stderr)
```

Key properties:

- The runtime process itself **never joins any new namespace**, so threading and
  tokio keep working.
- Stdio is handed to the child via `Stdio::from_raw_fd`; the parent never closes
  the write-ends manually (double-close hazard).
- Helper exit codes `125`/`126` are mapped to typed errors.
- `SpawnedProcess` supports `wait`, `try_wait`, `stop` (SIGTERM), `kill`
  (SIGKILL), `take_stdout`/`take_stderr`; its `Drop` SIGKILLs and reaps the
  container if nobody waited (no orphans).
- `pre_exec` in the child runs, in order: `unshare(CLONE_NEWNET)` (if `netns`),
  `unshare(CLONE_NEWNS)`, `prepare_container_root` (if a prep is given),
  `PR_SET_PDEATHSIG` (if `kill_on_parent_exit`), then `apply_security`.

`CommandSpec` is serializable and carries `program`, `args`, `env`,
`kill_on_parent_exit`, `security: Option<SecurityProfile>`, `netns`, `cwd`.

## 4. Filesystem

`src/filesystem/`:

- **`overlay.rs`** — `OverlaySpec { lower_layers, upper_dir, work_dir, target }`
  + `mount_overlay` (mount type `overlay`, `lowerdir=a:b:...`, `upperdir`,
  `workdir`).
- **`mounts.rs`** — `MountSpec { source, target, read_only }` + `bind_mount`;
  targets are resolved **relative to the container rootfs** (bind before
  pivot); `mount_devtmpfs` gives the container a usable `/dev`.
- **`pivot.rs`** — `pivot_root` sequence (Linux-only, `unsafe` scoped): mount
  `/.old_root`, `pivot_root`, `chdir("/")`, `umount2("/.old_root")`.
- **`rootfs.rs`** — `unpack_layer` (tar or tar.gz auto-detect by gzip magic)
  with **anti path-traversal** validation (`UnsafeLayer` on absolute or `..`
  entries); `sha256_file`.
- **`mod.rs`** — `RootfsPrep { overlay, bind_mounts }` and
  `prepare_container_root(prep)`: `mount --make-rprivate /` → `mount_overlay` →
  `mount_devtmpfs` → bind mounts → `pivot_root`.

The container's `/` is the overlay `merged` view: lower layers from the image
(read-only) plus a writable per-container `upper`/`work`.

## 5. Isolation

`src/isolation/`:

### cgroups v2 — `cgroups.rs`

- `ResourceLimits { cpu_quota: Option<f64>, memory_bytes: Option<u64>, pids_max: Option<u64> }`
  with `validate()`; `cpu_quota` is a fraction of a core → `cpu.max`
  `"<round(q*100000)> 100000"`.
- `CgroupManager` (base `/sys/fs/cgroup`): `create(name)` also **enables
  controllers** on the base via `cgroup.subtree_control` deltas (a plain child
  of an unconfigured cgroup has no `memory.max`/`pids.max` files); `apply`
  writes `cpu.max`, `memory.max`, `memory.swap.max` (equal to `memory.max`, so
  low limits trigger OOM instead of swapping), `pids.max`; `attach(pid)` writes
  `cgroup.procs`; `remove` writes `cgroup.kill` then rmdir.
- Errors use `RuntimeError::CgroupWrite { file, message }`.

### capabilities — `capabilities.rs`

- `Capability(u64)` / `CapabilitySet(u64)` bitmap; `default_allowlist()` =
  `CHOWN | DAC_OVERRIDE | FOWNER | SETGID | SETUID | NET_BIND_SERVICE`
  (mask `0x45b`).
- `restrict_to`: bounding-set drop (`PR_CAPBSET_DROP`) **first** (it requires
  `CAP_SETPCAP` effective, so it must run while still root), then a raw
  `capset` v3 syscall with a 24-byte data area (2 × `cap_data`).

### seccomp — `seccomp.rs`

- `SeccompSyscall` enum (23 syscalls) with a `default_denylist()` of 19
  (ptrace, process_vm_*, kexec_load, reboot, mount, umount2, pivot_root,
  chroot, module ops, bpf, userfaultfd, swap, acct, ioperm, iopl).
- Compiled to BPF with `seccompiler` and applied with default action `Allow`,
  blocked action `Errno(EPERM)`.

### privilege drop — `privdrop.rs`

- `ContainerUser { uid, gid }`; `drop_privileges` = `setgroups(&[])` →
  `setgid` → `setuid` (irreversible).

### composition — `mod.rs`

- `SecurityProfile { capabilities, seccomp, user }` and
  `apply_security(profile)` in this fixed order:
  `no_new_privs` → `apply_seccomp` → `restrict_to` → `drop_privileges`.
  NNP must precede seccomp for the filter to survive `exec`.

## 6. Network

`src/network/`:

- **`netlink.rs`** (Linux) — hand-rolled rtnetlink ABI over a raw
  `AF_NETLINK`/`NETLINK_ROUTE` socket (no external netlink crate): veth pairs,
  bridge create/delete, link up/master/move/rename, address add.
- **`bridge.rs`** — `db0` on `10.20.0.0/24`, gateway `10.20.0.1`; deterministic
  per-container IP = SHA-256(id) → host byte in `2..=251`; veth host name ≤ 15
  chars; `ensure_bridge` idempotent.
- **`namespace.rs`** (Linux) — `attach_container`: create veth, move the peer
  into the container netns, rename it to `eth0` inside the ns (via a thread
  that calls `setns`), bring it up, assign the address; `detach_container`
  deletes the host veth. The runtime's own threads/namespaces are untouched
  (`with_netns`).
- **`port.rs`** — **userland port publishing** (default path, no iptables):
  one tokio runtime per `PortMap`; TCP via accept loop + `copy_bidirectional`
  with upstream connect retries (5 attempts, capped backoff 200ms→3.2s); UDP
  via per-client connected sockets with an mpsc channel + `select!`.
  `PortMap` validation rejects zero/duplicate host ports.
- **`iptables.rs`** — optional (`feature = "iptables"`): direct DNAT + FORWARD
  rules via the `iptables(8)` binary.

## 7. Storage

`src/storage/`:

- **`volume.rs`** — `Volume { name, path }`; `VolumeStore` creates volumes with
  a `.tokedb-volume` marker file (distinguishes real volumes from orphan dirs),
  lists/gets/removes marker-verified volumes, and validates names via
  `state::validate_component`. `Volume::mount_spec(target, ro)` produces the
  bind `MountSpec` for the engine's `data_directory`.
- **`backup.rs`** — `VolumeLock` (O_EXCL lock file under
  `volumes/.locks/<name>.lock`, 20×100ms retry → `VolumeBusy`) and
  `backup_volume`: consistent tar snapshot via `append_dir_all` while holding
  the lock.

## 8. Execution flow (`src/runtime/run.rs`)

`start` (Linux):

1. Transition `Starting`, persist.
2. `build_rootfs`: unpack each image layer into `rootfs/lower{i}`, overlay
   `lower0:lower1:...` + `upper`/`work` → `merged`.
3. Bind system dirs read-only (`/bin`, `/usr/bin`, `/usr/lib[64]`,
   `/lib[64]`, `/etc` — the DB binaries and their dynamic deps come from the
   host) plus each volume bind at `data_directory`.
4. `chown_tree` the writable layers/bind sources to the container user
   (uid/gid 999), so the unprivileged DB process can write.
5. `spawn_with_prep`; then `wire_up`: `ensure_bridge` → `attach_container` →
   `spawn_port_proxies` → create/apply/attach a cgroup if any limit is set. On
   wire-up failure with a live pid: kill, reap, cleanup, persist `Stopped`.
6. Persist `Running` + pid; stream stdout/stderr to
   `containers/<id>/logs/{stdout,stderr}.log` (mirrored to the terminal) on
   background threads; block on `process.wait()`.
7. Persist `Stopped`, cleanup (delete veth, drop proxies, remove cgroup).

`stop` (Linux): require `Running` + pid → transition `Stopping` → SIGTERM, wait
≤ 10s → SIGKILL, wait ≤ 5s → persist `Stopped`. `ESRCH` is tolerated.

The engine security profile (`container_security`) is always applied:
default capability allowlist + seccomp denylist + drop to uid/gid 999 — a
database process **never runs as root**.