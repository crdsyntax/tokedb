import { useEffect, useState } from "react";
import type { EngineInfo } from "../types";
import { api } from "../api";

export function SettingsView() {
  const [dataRoot, setDataRoot] = useState("");
  const [engines, setEngines] = useState<EngineInfo[]>([]);

  useEffect(() => {
    api.getDataRoot().then(setDataRoot).catch(() => {});
    api.getEngineInfo().then(setEngines).catch(() => {});
  }, []);

  return (
    <div className="p-6 space-y-6 animate-fade-in">
      <div>
        <h2 className="text-xl font-bold text-white">Configuración</h2>
        <p className="text-sm text-dark-400">Información del runtime tokedb</p>
      </div>

      {/* Data root */}
      <div className="card space-y-3">
        <h3 className="text-sm font-semibold text-white">Data Root</h3>
        <div className="bg-dark-900 rounded-lg px-4 py-3">
          <code className="text-sm text-primary-400 font-mono">{dataRoot || "..."}</code>
        </div>
        <p className="text-xs text-dark-400">
          Variable de entorno: <code className="text-dark-200">TOKEDB_DATA_ROOT</code>
        </p>
        <div className="bg-dark-700/50 rounded-lg p-3 text-xs text-dark-400 space-y-1">
          <p>Subdirectorios:</p>
          <p className="ml-2">├── <code>images/</code> — Imágenes importadas</p>
          <p className="ml-2">├── <code>containers/</code> — Estado de contenedores</p>
          <p className="ml-2">├── <code>volumes/</code> — Volúmenes de datos persistentes</p>
          <p className="ml-2">└── <code>registry/</code> — Registry local</p>
        </div>
      </div>

      {/* Supported engines */}
      <div className="card space-y-3">
        <h3 className="text-sm font-semibold text-white">Motores soportados</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
          {engines.map((eng) => (
            <div key={eng.engine} className="bg-dark-700/50 rounded-lg p-3 space-y-1">
              <div className="flex items-center gap-2">
                <span className="text-lg">
                  {eng.engine === "mariadb" || eng.engine === "mysql"
                    ? "🐬"
                    : eng.engine === "postgres"
                    ? "🐘"
                    : "🍃"}
                </span>
                <span className="text-sm font-medium text-white">{eng.name}</span>
                <code className="text-xs text-dark-400">({eng.engine})</code>
              </div>
              <div className="text-xs text-dark-400 space-y-0.5 ml-7">
                <p>Puerto: <span className="text-dark-200">{eng.default_port}</span></p>
                <p>Data dir: <code className="text-dark-200">{eng.data_directory}</code></p>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Environment variables */}
      <div className="card space-y-3">
        <h3 className="text-sm font-semibold text-white">Variables de entorno</h3>
        <div className="bg-dark-700/50 rounded-lg p-3 space-y-2 text-xs">
          <div>
            <code className="text-primary-400">TOKEDB_DATA_ROOT</code>
            <span className="text-dark-400 ml-2">— Ruta del data root (default: /var/lib/db-runtime)</span>
          </div>
          <div>
            <code className="text-primary-400">RUST_LOG</code>
            <span className="text-dark-400 ml-2">— Nivel de logging (default: info)</span>
          </div>
          <div>
            <code className="text-primary-400">TOKEDB_WSL_DISTRO</code>
            <span className="text-dark-400 ml-2">— Distro WSL en Windows (default: Ubuntu-24.04)</span>
          </div>
        </div>
      </div>

      {/* About */}
      <div className="card space-y-2">
        <h3 className="text-sm font-semibold text-white">Acerca de</h3>
        <p className="text-xs text-dark-400">
          tokedb Manager v0.1.0 — Interfaz gráfica para el runtime de bases de datos tokedb.
        </p>
        <p className="text-xs text-dark-400">
          No es Docker. tokedb usa primitivas del kernel Linux para aislamiento de procesos de BD.
        </p>
      </div>
    </div>
  );
}
