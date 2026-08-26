import gzip, hashlib, io, json, os, sys, tarfile, time

ENGINES = {
    "mongodb":  {"version": "7.0",  "port": 27017, "data": "/data/db"},
    "postgres": {"version": "16",   "port": 5432, "data": "/var/lib/postgresql/data"},
    "redis":    {"version": "7.2",  "port": 6379, "data": "/data"},
    "sql":      {"version": "2022", "port": 1433, "data": "/var/opt/mssql/data"},
    "mysql":    {"version": "8.0",  "port": 3306, "data": "/var/lib/mysql"},
    "sqlite":   {"version": "3.45", "port": 54321, "data": "/var/lib/sqlite"},
}


def add_bytes(t, name, payload):
    info = tarfile.TarInfo(name)
    info.size = len(payload)
    info.mtime = int(time.time())
    t.addfile(info, io.BytesIO(payload))


def add_dir(t, name, mode=0o755):
    info = tarfile.TarInfo(name)
    info.type = tarfile.DIRTYPE
    info.mode = mode
    info.mtime = int(time.time())
    t.addfile(info, None)


def add_dirs(t, path):
    p = path.lstrip("/")
    if not p:
        return
    for i in range(1, len(p.split("/")) + 1):
        add_dir(t, "/".join(p.split("/")[:i]))


def build_layer(data_dir):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as t:
        add_dirs(t, "tmp")
        add_dir(t, "tmp", 0o1777)
        add_dirs(t, "run")
        add_dirs(t, "var/run")
        add_dirs(t, data_dir)
        add_bytes(t, data_dir.lstrip("/") + "/.tokedb-test", b"tokedb test engine fixture\n")
    return gzip.compress(buf.getvalue())


def make_bundle(engine, spec, out_dir):
    layer = build_layer(spec["data"])
    digest = "sha256:" + hashlib.sha256(layer).hexdigest()
    startup = ["python3", "-m", "http.server", str(spec["port"]), "--directory", spec["data"]]
    manifest = {
        "database": engine,
        "version": spec["version"],
        "architecture": "amd64",
        "default_port": spec["port"],
        "data_directory": spec["data"],
        "healthcheck": {"port": spec["port"], "timeout_secs": 5},
        "startup_command": startup,
        "layers": [{"digest": digest, "size": len(layer)}],
    }
    canonical = json.dumps(
        {k: v for k, v in manifest.items() if k != "digest"},
        sort_keys=True,
        separators=(",", ":"),
    )
    manifest["digest"] = "sha256:" + hashlib.sha256(canonical.encode()).hexdigest()
    hex_layer = hashlib.sha256(layer).hexdigest()

    out = os.path.join(out_dir, f"{engine}-{spec['version']}.tar.gz")
    with tarfile.open(out, "w:gz") as t:
        mj = json.dumps(manifest, sort_keys=True).encode()
        mi = tarfile.TarInfo("manifest.json")
        mi.size = len(mj)
        t.addfile(mi, io.BytesIO(mj))
        li = tarfile.TarInfo(f"layers/{hex_layer}.tar.gz")
        li.size = len(layer)
        t.addfile(li, io.BytesIO(layer))
    print("generado:", out)


if __name__ == "__main__":
    out_dir = sys.argv[1] if len(sys.argv) > 1 else "test-images"
    os.makedirs(out_dir, exist_ok=True)
    for engine, spec in ENGINES.items():
        make_bundle(engine, spec, out_dir)
