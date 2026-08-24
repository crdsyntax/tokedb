import gzip, hashlib, io, json, tarfile, time
def add_bytes(t: tarfile.TarFile, name: str, payload: bytes):
    info = tarfile.TarInfo(name)
    info.size = len(payload)
    info.mtime = int(time.time())
    t.addfile(info, io.BytesIO(payload))
def add_dir(t: tarfile.TarFile, name: str, uid: int = 0, gid: int = 0):
    info = tarfile.TarInfo(name)
    info.type = tarfile.DIRTYPE
    info.mode = 0o1777 if name == "tmp" else 0o755
    info.uid = uid
    info.gid = gid
    info.mtime = int(time.time())
    t.addfile(info, None)

buf = io.BytesIO()
with tarfile.open(fileobj=buf, mode="w") as t:
    add_bytes(
        t,
        "etc/mysql/my.cnf",
        b"[mysqld]\n"
        b"datadir=/var/lib/mysql\n"
        b"socket=/tmp/mysqld.sock\n"
        b"pid-file=/tmp/mysqld.pid\n"
        b"bind-address=0.0.0.0\n",
    )
    for d in ["tmp", "run", "var/run", "var/lib/mysql", "var/log/mysql"]:
        add_dir(t, d)
    add_dir(t, "run/mysqld", uid=999, gid=999)
    add_dir(t, "var/log/mysql", uid=999, gid=999)
layer = gzip.compress(buf.getvalue())
digest = "sha256:" + hashlib.sha256(layer).hexdigest()

manifest = {
    "database": "mariadb",
    "version": "10.11",
    "architecture": "amd64",
    "default_port": 3306,
    "data_directory": "/var/lib/mysql",
    "healthcheck": {"port": 3306, "timeout_secs": 5},
    "startup_command": ["mariadbd"],
    "layers": [{"digest": digest, "size": len(layer)}],
}
canonical = json.dumps(
    {k: v for k, v in manifest.items() if k != "digest"},
    sort_keys=True,
    separators=(",", ":"),
)
manifest["digest"] = "sha256:" + hashlib.sha256(canonical.encode()).hexdigest()

hex_layer = hashlib.sha256(layer).hexdigest()
with tarfile.open("/tmp/mariadb-10.11.tar.gz", "w:gz") as t:
    mj = json.dumps(manifest, sort_keys=True).encode()
    mi = tarfile.TarInfo("manifest.json")
    mi.size = len(mj)
    t.addfile(mi, io.BytesIO(mj))
    li = tarfile.TarInfo(f"layers/{hex_layer}.tar.gz")
    li.size = len(layer)
    t.addfile(li, io.BytesIO(layer))
print("bundle listo: /tmp/mariadb-10.11.tar.gz")
