# tokedb

**No es Docker.** tokedb es un *runtime de bases de datos*: un runtime de
procesos aislados en Rust especializado **exclusivamente** en ejecutar motores
de bases de datos (MariaDB, MySQL, PostgreSQL, MongoDB). No gestiona
contenedores genéricos ni imágenes arbitrarias — solo imágenes de estos motores.

Trabaja directamente con primitivas del kernel Linux (namespaces, cgroups v2,
overlayfs, pivot_root, capabilities, seccomp), con un formato de imagen propio
pensado para bases de datos (sin Dockerfiles, sin layers arbitrarias) y expone
los puertos mediante proxies userland (sin iptables por defecto).

### Motores soportados

El runtime conoce la especificación de cada motor (puerto, `data_directory`,
comando de arranque, usuario interno) y la valida en todo momento:

| Motor | Puerto | `data_directory` | Comando |
|---|---|---|---|
| `mariadb` | 3306 | `/var/lib/mysql` | `mariadbd` |
| `mysql` | 3306 | `/var/lib/mysql` | `mysqld` |
| `postgres` | 5432 | `/var/lib/postgresql/data` | `postgres` |
| `mongodb` | 27017 | `/data/db` | `mongod` |

Una imagen `tokedb` describe *un* motor (`database`, `version`, `default_port`,
`data_directory`, `startup_command`, `healthcheck`) — no un sistema operativo ni
una app genérica.

## Requisitos

- **Linux** (o WSL2 en Windows) para ejecutar contenedores. Los comandos
  `start` / `stop` requieren **root** (privilegios para namespaces, cgroups,
  mounts y netlink).
- **Rust toolchain** (edition 2021, rust-version 1.77.2) para compilar.
- En Windows, el binario actúa como cliente delgado y reenvía la invocación a
  Linux dentro de WSL2 (ver [Windows](#windows)).

## Compilar y ejecutar

Desde la raíz del workspace:

```sh
cargo build --release
# binario en: target/release/tokedb

# comprobar la CLI
cargo dev --help
# o directamente:
target/release/tokedb --help
```

En desarrollo existe el alias `cargo dev` (`cargo run --bin tokedb --`).

## Guía rápida (paso a paso)

Flujo completo de 5 pasos, del binario a una DB corriendo.

### 1. Preparar el entorno

Linux (o WSL2) con **root** para `start`/`stop`. Instala en el host el motor
que vayas a usar (sus binarios y librerías se montan en el contenedor desde el
host) y define el *data root*:

```sh
sudo apt install mariadb-server        # o mysql-server / postgresql / mongodb
export TOKEDB_DATA_ROOT=/var/lib/db-runtime
```

### 2. Levantar el runtime

No hay daemon: el runtime es el binario. Verifica que responde y que crea el
estado inicial:

```sh
tokedb --help        # subcomandos disponibles
tokedb images        # lista vacía → data_root/{images,containers,volumes}
```

### 3. Cargar una imagen

```sh
tokedb import mariadb-11.4.tar.gz      # bundle local
# o
tokedb pull mariadb:11.4               # registry local/remoto
tokedb images                          # verifica que quedó importada
```

(Generar un bundle a mano: ver [Crear un bundle](#crear-un-bundle-ejemplo).)

### 4. Configurar el contenedor

```sh
tokedb create mi-db mariadb:11.4 \
  --memory-mb 4096 --cpu-quota 2.0 --pids-max 100 \
  --port 3306                          # o --port 18080:3306
```

`create` genera el volumen de datos `mi-db-data` montado en el
`data_directory` del motor. Configuración del motor (my.cnf, postgresql.conf,
mongod.conf…) → archivo en el volumen antes de arrancar:

```sh
echo -e '[mysqld]\nmax_connections=500' \
  > $TOKEDB_DATA_ROOT/volumes/mi-db-data/my.cnf
```

### 5. Activar

```sh
sudo tokedb start mi-db        # root; construye rootfs, monta volumen+red+cgroups
                               # y ejecuta mariadbd EN PRIMER PLANO (bloquea)
```

En otra terminal, mientras corre: `tokedb list`, `tokedb inspect mi-db`,
`tokedb logs mi-db`. Para apagar: `tokedb stop mi-db`; para eliminar el
contenedor conservando datos: `tokedb destroy mi-db` (el volumen `mi-db-data`
se conserva).

> Resumen: **importar/pull → create (configura) → start (activa) → stop →
> destroy**. Los pasos detallados en las secciones siguientes.

### Ubicación de los datos

`tokedb` guarda todo su estado en un *data root* configurable con la variable
de entorno `TOKEDB_DATA_ROOT`:

| Variable | Default |
|---|---|
| `TOKEDB_DATA_ROOT` | `/var/lib/db-runtime` (Linux) / `.db-runtime` (otros) |
| `RUST_LOG` | `info` (logging) |

```
<data_root>/
├── images/       # imágenes importadas: <database>/<version>/{manifest.json, layers/}
├── containers/   # contenedores: <id>/{metadata.json, logs/, rootfs/}
├── volumes/      # volúmenes de datos (persistentes)
└── registry/     # registry local opcional: index.json + blobs/
```

## Agregar una imagen de base de datos

Una imagen `tokedb` es la definición de **un motor de base de datos**: un
**bundle** (tar.gz) con `manifest.json` y una o más capas
`layers/<sha256>.tar.gz`. El manifest identifica el motor (`database`) y cómo
arranca y guarda sus datos; las capas aportan el resto del rootfs. No hay
Dockerfiles ni imágenes de propósito general. Para tener una imagen en el
store puedes **importar** un bundle o **descargarla** de un registry.

### Importar un bundle

```sh
tokedb import mariadb-11.4.tar.gz
# importado mariadb:11.4 (1 layer(s), digest sha256:...)
```

### Descargar de un registry

```sh
# desde un registry local (directorio) o el default
tokedb pull mariadb:11.4
tokedb pull mariadb:11.4 --registry ./mi-registry

# desde un registry remoto (subset de Registry API v2)
tokedb pull mariadb:11.4 --registry https://registry.example.com
```

`pull` descarga a un directorio temporal y **verifica digests** antes de
importar: un manifest corrupto o con digest incorrecto se rechaza con un error
tipado (`DigestMismatch`, `CorruptImage`).

### Ver las imágenes disponibles

```sh
tokedb images
# mariadb:11.4  mariadb:11.4  amd64  sha256:...  1 layer(s)
```

### Crear un bundle (ejemplo)

El bundle describe **el motor**: qué motor es (`database`), dónde viven sus
datos (`data_directory`), su puerto, el comando de arranque y las capas del
rootfs. Ejemplo mínimo con Python (JSON canónico con claves ordenadas y el
campo `digest` fuera del hash):

```python
import gzip, hashlib, io, json, tarfile

def make_layer(payload: bytes) -> bytes:
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as t:
        info = tarfile.TarInfo("usr/local/tokedb/db")
        info.size = len(payload)
        t.addfile(info, io.BytesIO(payload))
    return gzip.compress(buf.getvalue())

layer = make_layer(b"hello db")
digest = "sha256:" + hashlib.sha256(layer).hexdigest()
manifest = {
    "database": "mariadb",            # motor: mariadb|mysql|postgres|mongodb
    "version": "11.4",                # tag
    "architecture": "amd64",          # amd64|arm64
    "default_port": 3306,             # puerto del motor dentro del contenedor
    "data_directory": "/var/lib/mysql", # donde viven los datos del motor
    "healthcheck": {"port": 3306, "timeout_secs": 5},
    "startup_command": ["mariadbd"],  # programa + argumentos
    "layers": [{"digest": digest, "size": len(layer)}],
}
canonical = json.dumps(
    {k: v for k, v in manifest.items() if k != "digest"},
    sort_keys=True, separators=(",", ":"),
)
manifest["digest"] = "sha256:" + hashlib.sha256(canonical.encode()).hexdigest()

hex_layer = hashlib.sha256(layer).hexdigest()
with tarfile.open("mariadb-11.4.tar.gz", "w:gz") as t:
    mj = json.dumps(manifest, sort_keys=True).encode()
    mi = tarfile.TarInfo("manifest.json"); mi.size = len(mj)
    t.addfile(mi, io.BytesIO(mj))
    li = tarfile.TarInfo(f"layers/{hex_layer}.tar.gz"); li.size = len(layer)
    t.addfile(li, io.BytesIO(layer))
```

> Nota: los binarios y librerías del motor se montan desde el host
> (`/bin`, `/usr/bin`, `/usr/lib`, `/etc`, ...) en read-only; las capas de la
> imagen aportan el resto del rootfs del contenedor. Una imagen real incluye
> también la configuración por defecto del motor.

## Ejecutar el contenedor de una imagen

El ciclo de vida es `create → start → stop → destroy`, más `logs`, `inspect`
y `list`. La imagen debe estar ya importada (ver sección anterior).

### Crear

```sh
tokedb create mi-db mariadb:11.4
# created container `mi-db` (id ab12cd34) with data volume `mi-db-data`
```

Al crear se genera automáticamente un **volumen de datos** `<nombre>-data`
montado en el `data_directory` de la imagen (persistente: sobrevive al
`destroy` del contenedor).

### Arrancar

```sh
tokedb start mi-db
```

`start` construye el rootfs (overlayfs a partir de las capas), monta el
volumen y los binds del sistema, configura red y cgroups, y ejecuta el
`startup_command` de la imagen **en primer plano**: verás el log del motor en
la terminal y el comando queda bloqueado mientras el contenedor corre.

### Operaciones

```sh
tokedb list                       # id  name  image  state  pid
tokedb inspect mi-db              # metadata.json formateado
tokedb logs mi-db                 # stdout.log + stderr.log capturados
tokedb stop mi-db                 # SIGTERM y SIGKILL tras la gracia (10s/5s)
tokedb destroy mi-db              # elimina el contenedor; conserva mi-db-data
```

## Configurar la base de datos en el contenedor

Hay dos niveles de configuración: la **imagen** (manifest) y el **contenedor**
(flags de `create`).

### Configuración de la imagen (manifest)

El manifest define el comportamiento por defecto de la base de datos:

| Campo | Qué controla |
|---|---|
| `database` / `version` | Motor y versión (forman la referencia `motor:version`) |
| `default_port` | Puerto dentro del contenedor (`create` lo publica con `--port`) |
| `data_directory` | Dónde se monta el volumen de datos (persistencia de la DB) |
| `startup_command` | Binario y argumentos que arranca `start` |
| `healthcheck` | Puerto y timeout del healthcheck |
| `layers` | Contenido del rootfs (content-addressed por sha256) |

### Configuración del contenedor (`create`)

```sh
tokedb create mi-db mariadb:11.4 \
  --memory-mb 4096 \        # límite de RAM (cgroup memory.max)
  --cpu-quota 2.0 \         # núcleos CPU (cpu.max, fracción de core)
  --pids-max 100 \          # límite de procesos (pids.max)
  --port 3306 \             # publica 3306 (host:contenedor iguales)
  --port 18080:3306         # publica el 3306 del contenedor en el host:18080
```

- `--port` acepta `HOST:CONTAINER` o un solo `PORT` (igual en ambos lados).
  El puerto del host es alcanzable desde la máquina anfitriona mediante un
  proxy userland (TCP).
- Los límites de recursos se imponen con cgroups v2 (no hay swap extra:
  `memory.swap.max` = `memory.max`).
- Los puertos duplicados y los puertos `0` se rechazan con error tipado.

### Configuración de la base de datos (datos y archivos de config)

El volumen `<nombre>-data` está montado en el `data_directory` del motor con
permiso de escritura (propiedad uid/gid 999, nunca root). Para personalizar el
motor (p. ej. un `my.cnf` para MariaDB/MySQL, `postgresql.conf` para
PostgreSQL, `mongod.conf` para MongoDB), coloca el archivo en el volumen antes
de `start`; los datos persistirán entre `stop`/`start` y sobrevivirán al
`destroy` del contenedor:

```sh
# ejemplo: colocar config en el volumen de datos del contenedor mi-db
echo '[mysqld]\nmax_connections=500' \
  > .db-runtime/volumes/mi-db-data/my.cnf
```

> Nota: el contenedor ejecuta el `startup_command` de la imagen tal cual. Si el
> motor necesita flags de arranque, deben venir en el manifest de la imagen
> (`startup_command`) o vía archivos de config en el volumen.

## Mejoras recomendadas al flujo

Priorizadas por impacto; identificadas contra el comportamiento actual del
código y pendientes de implementar:

| # | Mejora | Por qué | Costo |
|---|---|---|---|
| 1 | `start --detach` (segundo plano) | Hoy `start` bloquea la terminal y `kill_on_parent_exit` mata la DB si cierras la terminal. Uso real necesita modo daemon con log solo a `stdout.log`/`stderr.log`. | Alto |
| 2 | Wait-for-ready del `healthcheck` | El manifest define `healthcheck {port, timeout}` pero el runtime no lo usa: `start` no espera a que la DB acepte conexiones antes de reportar "running". | Medio |
| 3 | Validar el motor al importar | El runtime solo soporta 4 motores (`database::for_engine`) pero no lo aplica: `import`/`pull` aceptan cualquier `database`. Debe rechazar motores desconocidos en `import_staged` (`InvalidManifest`). | Bajo |
| 4 | `--env` y argumentos en `create` | `CommandSpec` ya soporta `env` y `args`, pero `create` no los expone; hoy la config del motor es manual (archivos en el volumen). | Medio |
| 5 | `logs --follow` real | El flag `-f` se parsea pero `run::logs` solo imprime los archivos; implementar tail en vivo o quitarlo. | Bajo |
| 6 | `tokedb push` al registry local | Hay `pull` pero no `push` (publish es solo API); un `push` cierra el ciclo publicar/consumir. | Bajo |
| 7 | Auto-inicialización del data root | `StateLayout::ensure_directories` existe pero no se llama en el CLI; cada store crea sus dirs por su cuenta. | Bajo |
| 8 | Documentar/resolver binarios desde el host | Los binarios del motor se montan desde el host, no vienen de la imagen; es contraintuitivo para un formato de imagen propio. Mover resolución a las capas o documentar fuerte. | Alto |

## Windows

En Windows el binario `tokedb` valida la línea de comandos localmente y
reenvía la invocación a Linux vía WSL2:

| Variable | Default |
|---|---|
| `TOKEDB_WSL_DISTRO` | `Ubuntu-24.04` |
| `TOKEDB_WSL_BINARY` | `/usr/local/bin/tokedb` |
| `TOKEDB_DATA_ROOT` | (heredado, traducido a `/mnt/...`) |

Dentro de WSL2 deben estar instalados el binario `tokedb` en la ruta indicada
y los requisitos de la sección [Requisitos](#requisitos).

## Verificación del proyecto

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test                      # unit tests (cualquier SO)
cargo test --features integration-linux   # integración Linux (root, WSL2)
```

## Documentación

Detalle de arquitectura, formato de imagen, runtime y CLI en `docs/`
(`docs/README.md` es el índice). El plan por fases y su estado en
`IMPLEMENTATION_PLAN.md`; el registro de decisiones y hallazgos en `notes.md`.

## Plan de desarrollo: interfaz de consola

Objetivo: una **consola interactiva** (`tokedb console`) que permita manejar
todo el sistema desde un único lugar — imágenes, contenedores, volúmenes,
registries y configuración — con el mismo nivel de rigor que el CLI.

### C0 — Fundación de la consola ✅

- `src/console/mod.rs`: REPL con prompt `tokedb>`, historial y edición
  (rustyline), `help`/`exit`, parser con argumentos tipados y validación en el
  borde.
- **Refactor previo** ✅: la lógica se extrajo a `src/service/mod.rs`
  (`RuntimeService` sobre los stores); el CLI (`cli::run_with_config`) y la
  consola (`console::execute`) consumen el mismo servicio — sin duplicar lógica
  ni divergir en errores.
- Dependencias: `rustyline` (line editor + historial, funciona en Windows).
- Criterios cumplidos: REPL arranca, `help`/`exit`, comandos parsean con
  argumentos tipados; fmt/clippy/test verdes en Windows (161 tests).

### C1 — Comandos del sistema ✅

- Los 12 comandos del CLI mapeados a la consola:
  `pull`, `import`, `export`, `images`, `rmi`, `create`, `start`, `stop`,
  `logs`, `inspect`, `destroy`, `list`.
- Criterios cumplidos: paridad funcional con el CLI; suite de tests del parser
  (`tokenize`, `help`/`exit`, errores de uso) y de cada comando sobre un data
  root temporal (import/images/rmi y create/list/inspect/destroy roundtrip).
- Pendiente (no crítico): tab-completion por subcomando y tablas dinámicas.

### C2 — Supervisión en vivo ✅

- `watch list [--interval <secs>]`: refresco periódico (por defecto 2s) del
  estado de contenedores con colores por estado (running verde, transiciones
  ámbar, stopped/destroyed rojo, created cian); limpia pantalla en TTY; sale con
  Ctrl+C (`ctrlc` + flag atómico compartido con la consola).
- `logs <name> [-f|--follow]`: tail en vivo por deltas (`delta_lines` compara
  sufijos de stdout+stderr entre sondeos; tolera truncamiento/rotación).
- Runtime: `run::read_logs` + `ContainerLogs { stdout, stderr }` (re-exportado),
  expuesto en el servicio como `read_logs` para que el CLI/consola no toquen el
  layout de logs.
- Criterios cumplidos: `watch` refleja `Running → Stopped` al hacer `stop` (la
  misma capa de servicio); tests de `delta_lines`, `follow_logs` y errores de
  uso de `watch` sobre data root temporal (165 tests en Windows).
- Pendiente (mejora #5): seguimiento real por inotify en lugar de sondeo.

### C3 — Acciones asíncronas ✅

- `pull` y `stop` corren en un thread de background con spinner
  (`spinner_wait`: frames braille en TTY, `title ...` sin TTY) y notifican al
  terminar; `RuntimeService` ahora es `Clone` para que los threads usen una
  copia.
- `start <name>` se lanza en background (`run_detached`): valida la existencia
  del contenedor de forma síncrona (fail fast), devuelve el prompt de inmediato
  y notifica `[done] container <name> stopped` cuando termina.
- Sin huérfanos: registro `PENDING` de tareas en vuelo (la consola avisa al
  salir si quedan pendientes) y `kill_on_parent_exit` (run.rs) mata la cadena
  de procesos del contenedor si el proceso de la consola muere.
- Criterios cumplidos: `start` devuelve el prompt, la consola avisa cuando el
  contenedor termina, sin huérfanos (tests de `spinner_wait`, `run_detached` y
  el registro PENDING; 170 tests en Windows).
- Pendiente (mejora #1): `start --detach` a nivel runtime para el CLI.

### C4 — Volúmenes y registries ✅

- `volume list/create <name>/remove <name>/backup <name> <dest-dir>` sobre
  `VolumeStore` + `backup_volume`; `volume backup` adquiere el `VolumeLock` y
  falla tipado con `VolumeBusy` si el volumen está ocupado (criterio ✅).
- `registry list` (nuevo `LocalRegistry::list`) y
  `registry publish <reference> [--registry <path>]` — mejora #6: push al
  registry local (default `<data_root>/registry`), simétrico a `pull`.
- `config show`: data root, rutas derivadas (images/containers/volumes) y
  bridge.
- Capa de servicio: `volume_*`, `registry_list`, `registry_publish`.
- Criterios cumplidos: backup bloqueado bajo lock (test `VolumeBusy`);
  roundtrips de volúmenes y registry sobre data root temporal (173 tests en
  Windows).

### C5 — Scripting y modo no interactivo ✅

- `tokedb console -c "create mi-db mariadb:11.4; start mi-db"` — batch de una
  sola ejecución que reutiliza exactamente el parser y errores de la consola
  (`split_commands` separa por `;` fuera de comillas; `run_batch` corta en el
  primer error y propaga el exit code tipado).
- Modo stdin: si `tokedb console` recibe pipes (stdin no TTY), lee líneas y las
  ejecuta con el mismo parser (`run_stdin_batch`).
- `--json` (flag global) emite DTOs tipados serde en vez de tablas:
  `images --json` → `Vec<ImageSummary>`, `list --json` → `Vec<Container>`
  (sin mapas dinámicos), reutilizable por toketeo vía IPC.
- Criterios cumplidos: batch reusa parser y errores; `--json` emite DTOs (177
  tests en Windows; `console -c` y `--json` se parsean localmente en Windows
  antes del reenvío a WSL2).

### C6 — Integración con toketeo

- La consola reutiliza el mismo `runtime_service` que expondrá la fase F9;
  toketeo no duplica lógica de consola (los commands IPC llaman a la lib).
- Criterios: una sola implementación de use-cases; consola y commands no
  divergen.

### Estándares

Iguales que el resto del proyecto: tipado fuerte en todas las fronteras,
`Result<T, RuntimeError>`, validación en el borde, `#![deny(unsafe_code)]` sin
nuevos módulos `unsafe`, una fase = un cambio atómico con
`cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings` y
`cargo test` verdes; integración Linux en WSL2 con
`cargo test --features integration-linux`.

### Riesgos

- **Windows**: la consola interactiva corre en Windows pero el runtime real
  vive en WSL2; la consola debe operar igual que el CLI (cliente delgado) o
  conectarse al runtime en la distro.
- **TUI complejo**: empezar por un REPL (reedline) y dejar una TUI full-screen
  (ratatui) como fase futura opcional, evitando raw-mode prematuro.