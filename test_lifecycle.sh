#!/usr/bin/env bash
set -e
export TOKEDB_DATA_ROOT=/tmp/tokedb-test

echo "=== limpieza previa ==="
tokedb destroy mi-db 2>/dev/null || true
tokedb rmi mariadb:10.11 2>/dev/null || true

echo "=== generar bundle ==="
python3 /mnt/d/Documents/GitHub/tokedb/make_image.py

echo "=== import ==="
tokedb import /tmp/mariadb-10.11.tar.gz
echo "=== images ==="
tokedb images
echo "=== create (la config del motor viene de my.cnf en la imagen) ==="
tokedb create mi-db mariadb:10.11 --port 13306:3306

echo "=== init datadir (mariadb-install-db sobre el volumen) ==="
mariadb-install-db --datadir=/tmp/tokedb-test/volumes/mi-db-data --user=root --auth-root-authentication-method=normal 2>&1 | tail -3
chown -R 999:999 /tmp/tokedb-test/volumes/mi-db-data
echo "datadir listo:"; ls /tmp/tokedb-test/volumes/mi-db-data | head
echo "=== start (background) ==="
tokedb start mi-db &
START_PID=$!
sleep 15
echo "=== list (debe decir running) ==="
tokedb list
echo "=== logs ==="
tokedb logs mi-db | tail -20
echo "=== connect test (proxy 13306) ==="
python3 - <<'PY'
import socket
try:
    s = socket.create_connection(("127.0.0.1", 13306), timeout=5)
    s.settimeout(3)
    data = s.recv(64)
    print("CONNECT OK, server banner:", data[:25])
    s.close()
except Exception as e:
    print("CONNECT FAILED:", e)
PY
echo "=== stop ==="
tokedb stop mi-db
wait $START_PID 2>/dev/null || true
echo "=== list tras stop ==="
tokedb list
echo "=== destroy ==="
tokedb destroy mi-db
echo "=== rmi ==="
tokedb rmi mariadb:10.11
echo "=== fin ==="
