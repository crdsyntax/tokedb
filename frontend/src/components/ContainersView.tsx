import { useState } from "react";
import type { Container, ImageSummary } from "../types";
import { api } from "../api";

interface Props {
  containers: Container[];
  images: ImageSummary[];
  onRefresh: () => void;
}

const stateColors: Record<string, string> = {
  created: "badge-gray",
  starting: "badge-yellow",
  running: "badge-green",
  stopping: "badge-yellow",
  stopped: "badge-red",
  destroyed: "badge-red",
};

const engineIcons: Record<string, string> = {
  mariadb: "🐬",
  mysql: "🐬",
  postgres: "🐘",
  mongodb: "🍃",
};

export function ContainersView({ containers, images, onRefresh }: Props) {
  const [showCreate, setShowCreate] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [logs, setLogs] = useState<{ stdout: string; stderr: string } | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Create form state
  const [newName, setNewName] = useState("");
  const [newImage, setNewImage] = useState("");
  const [newMemory, setNewMemory] = useState("");
  const [newCpu, setNewCpu] = useState("");
  const [newPort, setNewPort] = useState("");

  const handleCreate = async () => {
    if (!newName || !newImage) return;
    try {
      await api.createContainer(newName, newImage, {
        memory_mb: newMemory ? parseInt(newMemory) : undefined,
        cpu_quota: newCpu ? parseFloat(newCpu) : undefined,
        ports: newPort ? [newPort] : undefined,
      });
      setShowCreate(false);
      setNewName("");
      setNewImage("");
      setNewMemory("");
      setNewCpu("");
      setNewPort("");
      onRefresh();
    } catch (e: any) {
      setError(String(e));
    }
  };

  const handleAction = async (name: string, action: "start" | "stop" | "destroy") => {
    setActionLoading(`${action}-${name}`);
    setError(null);
    try {
      if (action === "start") await api.startContainer(name);
      else if (action === "stop") await api.stopContainer(name);
      else if (action === "destroy") await api.destroyContainer(name);
      onRefresh();
    } catch (e: any) {
      setError(String(e));
    } finally {
      setActionLoading(null);
    }
  };

  const handleLogs = async (name: string) => {
    try {
      const l = await api.readLogs(name);
      setLogs(l);
      setSelected(name);
    } catch (e: any) {
      setError(String(e));
    }
  };

  const selectedContainer = containers.find((c) => c.name === selected);

  return (
    <div className="p-6 space-y-4 animate-fade-in">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold text-white">Contenedores</h2>
          <p className="text-sm text-dark-400">Gestionar ciclos de vida de las bases de datos</p>
        </div>
        <button onClick={() => setShowCreate(!showCreate)} className="btn-primary">
          + Crear contenedor
        </button>
      </div>

      {error && (
        <div className="bg-red-500/10 border border-red-500/30 rounded-lg px-4 py-2 text-sm text-red-400">
          {error}
          <button onClick={() => setError(null)} className="ml-2 text-red-300 hover:text-white">
            ✕
          </button>
        </div>
      )}

      {/* Create form */}
      {showCreate && (
        <div className="card animate-fade-in space-y-3">
          <h3 className="text-sm font-semibold text-white">Nuevo contenedor</h3>
          <div className="grid grid-cols-2 lg:grid-cols-4 gap-3">
            <div>
              <label className="text-xs text-dark-400 mb-1 block">Nombre *</label>
              <input
                className="input w-full"
                placeholder="mi-db"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
              />
            </div>
            <div>
              <label className="text-xs text-dark-400 mb-1 block">Imagen *</label>
              <select
                className="input w-full"
                value={newImage}
                onChange={(e) => setNewImage(e.target.value)}
              >
                <option value="">Seleccionar...</option>
                {images.map((img) => (
                  <option key={img.reference} value={img.reference}>
                    {img.reference}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className="text-xs text-dark-400 mb-1 block">Memoria (MB)</label>
              <input
                className="input w-full"
                type="number"
                placeholder="4096"
                value={newMemory}
                onChange={(e) => setNewMemory(e.target.value)}
              />
            </div>
            <div>
              <label className="text-xs text-dark-400 mb-1 block">CPU cores</label>
              <input
                className="input w-full"
                type="number"
                step="0.5"
                placeholder="2.0"
                value={newCpu}
                onChange={(e) => setNewCpu(e.target.value)}
              />
            </div>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-xs text-dark-400 mb-1 block">Puerto (HOST:CONTAINER)</label>
              <input
                className="input w-full"
                placeholder="3306:3306"
                value={newPort}
                onChange={(e) => setNewPort(e.target.value)}
              />
            </div>
          </div>
          <div className="flex gap-2">
            <button onClick={handleCreate} className="btn-primary">
              Crear
            </button>
            <button onClick={() => setShowCreate(false)} className="btn-ghost">
              Cancelar
            </button>
          </div>
        </div>
      )}

      {/* Container table */}
      {containers.length === 0 ? (
        <div className="card text-center py-12">
          <p className="text-dark-400">No hay contenedores creados</p>
          <button onClick={() => setShowCreate(true)} className="btn-primary mt-4">
            Crear primer contenedor
          </button>
        </div>
      ) : (
        <div className="card overflow-hidden !p-0">
          <table className="w-full">
            <thead>
              <tr className="border-b border-dark-600 text-xs text-dark-400">
                <th className="text-left px-4 py-2">Nombre</th>
                <th className="text-left px-4 py-2">Imagen</th>
                <th className="text-left px-4 py-2">Estado</th>
                <th className="text-left px-4 py-2">Puertos</th>
                <th className="text-left px-4 py-2">PID</th>
                <th className="text-right px-4 py-2">Acciones</th>
              </tr>
            </thead>
            <tbody>
              {containers.map((c) => (
                <tr
                  key={c.id}
                  className={`border-b border-dark-700 hover:bg-dark-700/50 cursor-pointer ${
                    selected === c.name ? "bg-dark-700/30" : ""
                  }`}
                  onClick={() => handleLogs(c.name)}
                >
                  <td className="px-4 py-2.5">
                    <span className="text-sm text-white font-medium">{c.name}</span>
                    <span className="text-xs text-dark-500 ml-2">{c.id}</span>
                  </td>
                  <td className="px-4 py-2.5">
                    <span className="text-sm text-dark-200">
                      {engineIcons[c.image.split(":")[0]] || ""} {c.image}
                    </span>
                  </td>
                  <td className="px-4 py-2.5">
                    <span className={stateColors[c.state]}>{c.state}</span>
                  </td>
                  <td className="px-4 py-2.5 text-xs text-dark-300">
                    {c.ports.map((p) => `${p.host_port}:${p.container_port}`).join(", ") || "—"}
                  </td>
                  <td className="px-4 py-2.5 text-xs text-dark-400 font-mono">
                    {c.pid ?? "—"}
                  </td>
                  <td className="px-4 py-2.5 text-right" onClick={(e) => e.stopPropagation()}>
                    <div className="flex gap-1 justify-end">
                      {c.state === "created" || c.state === "stopped" ? (
                        <button
                          onClick={() => handleAction(c.name, "start")}
                          disabled={actionLoading === `start-${c.name}`}
                          className="text-xs px-2 py-1 rounded bg-green-500/20 text-green-400 hover:bg-green-500/30 disabled:opacity-50"
                        >
                          ▶ Start
                        </button>
                      ) : c.state === "running" ? (
                        <button
                          onClick={() => handleAction(c.name, "stop")}
                          disabled={actionLoading === `stop-${c.name}`}
                          className="text-xs px-2 py-1 rounded bg-yellow-500/20 text-yellow-400 hover:bg-yellow-500/30 disabled:opacity-50"
                        >
                          ■ Stop
                        </button>
                      ) : null}
                      {(c.state === "created" || c.state === "stopped") && (
                        <button
                          onClick={() => handleAction(c.name, "destroy")}
                          disabled={actionLoading === `destroy-${c.name}`}
                          className="text-xs px-2 py-1 rounded bg-red-500/20 text-red-400 hover:bg-red-500/30 disabled:opacity-50"
                        >
                          🗑
                        </button>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Logs panel */}
      {selected && logs && (
        <div className="card animate-fade-in space-y-2">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-white">
              Logs — {selected}
            </h3>
            <button onClick={() => { setSelected(null); setLogs(null); }} className="text-xs text-dark-400 hover:text-white">
              ✕ Cerrar
            </button>
          </div>
          {selectedContainer && (
            <div className="flex gap-3 text-xs text-dark-400 mb-2">
              <span>Imagen: {selectedContainer.image}</span>
              <span>Estado: <span className={stateColors[selectedContainer.state]}>{selectedContainer.state}</span></span>
              {selectedContainer.pid && <span>PID: {selectedContainer.pid}</span>}
            </div>
          )}
          <div className="bg-dark-900 rounded-lg p-3 max-h-64 overflow-y-auto font-mono text-xs">
            {logs.stdout && (
              <div className="mb-2">
                <span className="text-dark-500">stdout:</span>
                <pre className="text-green-400 whitespace-pre-wrap">{logs.stdout}</pre>
              </div>
            )}
            {logs.stderr && (
              <div>
                <span className="text-dark-500">stderr:</span>
                <pre className="text-red-400 whitespace-pre-wrap">{logs.stderr}</pre>
              </div>
            )}
            {!logs.stdout && !logs.stderr && (
              <p className="text-dark-500">Sin logs disponibles</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
