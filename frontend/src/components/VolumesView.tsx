import { useState } from "react";
import type { Volume } from "../types";
import { api } from "../api";

interface Props {
  volumes: Volume[];
  onRefresh: () => void;
}

export function VolumesView({ volumes, onRefresh }: Props) {
  const [newName, setNewName] = useState("");
  const [backupDest, setBackupDest] = useState("");
  const [backupTarget, setBackupTarget] = useState<string | null>(null);
  const [loading, setLoading] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const handleCreate = async () => {
    if (!newName) return;
    setLoading("create");
    setError(null);
    try {
      await api.createVolume(newName);
      setMessage(`Volumen "${newName}" creado`);
      setNewName("");
      onRefresh();
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(null);
    }
  };

  const handleRemove = async (name: string) => {
    setLoading(`remove-${name}`);
    setError(null);
    try {
      await api.removeVolume(name);
      setMessage(`Volumen "${name}" eliminado`);
      onRefresh();
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(null);
    }
  };

  const handleBackup = async () => {
    if (!backupTarget || !backupDest) return;
    setLoading(`backup-${backupTarget}`);
    setError(null);
    try {
      const path = await api.backupVolume(backupTarget, backupDest);
      setMessage(`Backup de "${backupTarget}" → ${path}`);
      setBackupTarget(null);
      setBackupDest("");
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(null);
    }
  };

  return (
    <div className="p-6 space-y-4 animate-fade-in">
      <div>
        <h2 className="text-xl font-bold text-white">Volúmenes</h2>
        <p className="text-sm text-dark-400">Datos persistentes para bases de datos</p>
      </div>

      {error && (
        <div className="bg-red-500/10 border border-red-500/30 rounded-lg px-4 py-2 text-sm text-red-400">
          {error}
          <button onClick={() => setError(null)} className="ml-2 text-red-300 hover:text-white">✕</button>
        </div>
      )}
      {message && (
        <div className="bg-green-500/10 border border-green-500/30 rounded-lg px-4 py-2 text-sm text-green-400">
          {message}
          <button onClick={() => setMessage(null)} className="ml-2 text-green-300 hover:text-white">✕</button>
        </div>
      )}

      {/* Create volume */}
      <div className="card flex items-end gap-3">
        <div className="flex-1">
          <label className="text-xs text-dark-400 mb-1 block">Nuevo volumen</label>
          <input
            className="input w-full"
            placeholder="nombre-del-volumen"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleCreate()}
          />
        </div>
        <button
          onClick={handleCreate}
          disabled={!newName || loading === "create"}
          className="btn-primary"
        >
          {loading === "create" ? "Creando..." : "+ Crear"}
        </button>
      </div>

      {/* Volume list */}
      {volumes.length === 0 ? (
        <div className="card text-center py-12">
          <p className="text-dark-400">No hay volúmenes creados</p>
        </div>
      ) : (
        <div className="card overflow-hidden !p-0">
          <table className="w-full">
            <thead>
              <tr className="border-b border-dark-600 text-xs text-dark-400">
                <th className="text-left px-4 py-2">Nombre</th>
                <th className="text-left px-4 py-2">Ruta</th>
                <th className="text-right px-4 py-2">Acciones</th>
              </tr>
            </thead>
            <tbody>
              {volumes.map((vol) => (
                <tr key={vol.name} className="border-b border-dark-700 hover:bg-dark-700/50">
                  <td className="px-4 py-2.5">
                    <span className="text-sm text-white font-medium">💾 {vol.name}</span>
                  </td>
                  <td className="px-4 py-2.5 text-xs text-dark-400 font-mono">{vol.path}</td>
                  <td className="px-4 py-2.5 text-right">
                    <div className="flex gap-1 justify-end">
                      {backupTarget === vol.name ? (
                        <div className="flex gap-1 items-center">
                          <input
                            className="input text-xs py-1"
                            placeholder="destino"
                            value={backupDest}
                            onChange={(e) => setBackupDest(e.target.value)}
                            onKeyDown={(e) => e.key === "Enter" && handleBackup()}
                            autoFocus
                          />
                          <button
                            onClick={handleBackup}
                            disabled={!backupDest || loading === `backup-${vol.name}`}
                            className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30"
                          >
                            ✓
                          </button>
                          <button
                            onClick={() => { setBackupTarget(null); setBackupDest(""); }}
                            className="text-xs px-2 py-1 rounded bg-dark-600 text-dark-300"
                          >
                            ✕
                          </button>
                        </div>
                      ) : (
                        <>
                          <button
                            onClick={() => setBackupTarget(vol.name)}
                            className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30"
                          >
                            📦 Backup
                          </button>
                          <button
                            onClick={() => handleRemove(vol.name)}
                            disabled={loading === `remove-${vol.name}`}
                            className="text-xs px-2 py-1 rounded bg-red-500/20 text-red-400 hover:bg-red-500/30 disabled:opacity-50"
                          >
                            🗑
                          </button>
                        </>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
