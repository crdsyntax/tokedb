import { useState } from "react";
import type { ImageSummary } from "../types";
import { api } from "../api";

interface Props {
  images: ImageSummary[];
  onRefresh: () => void;
}

const engineIcons: Record<string, string> = {
  mariadb: "🐬",
  mysql: "🐬",
  postgres: "🐘",
  mongodb: "🍃",
};

export function ImagesView({ images, onRefresh }: Props) {
  const [pullRef, setPullRef] = useState("");
  const [pullRegistry, setPullRegistry] = useState("");
  const [importPath, setImportPath] = useState("");
  const [exportRef, setExportRef] = useState("");
  const [exportOutput, setExportOutput] = useState("");
  const [loading, setLoading] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handlePull = async () => {
    if (!pullRef) return;
    setLoading("pull");
    setError(null);
    try {
      const msg = await api.pullImage(pullRef, pullRegistry || undefined);
      setMessage(msg);
      onRefresh();
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(null);
    }
  };

  const handleImport = async () => {
    if (!importPath) return;
    setLoading("import");
    setError(null);
    try {
      const msg = await api.importImage(importPath);
      setMessage(msg);
      setImportPath("");
      onRefresh();
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(null);
    }
  };

  const handleExport = async () => {
    if (!exportRef || !exportOutput) return;
    setLoading("export");
    setError(null);
    try {
      await api.exportImage(exportRef, exportOutput);
      setMessage(`Exportada ${exportRef} → ${exportOutput}`);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(null);
    }
  };

  const handleRemove = async (ref: string) => {
    setLoading(`remove-${ref}`);
    setError(null);
    try {
      await api.removeImage(ref);
      onRefresh();
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(null);
    }
  };

  return (
    <div className="p-6 space-y-4 animate-fade-in">
      <div>
        <h2 className="text-xl font-bold text-white">Imágenes</h2>
        <p className="text-sm text-dark-400">Importar, descargar y gestionar imágenes de motores de BD</p>
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

      {/* Pull / Import / Export actions */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        {/* Pull */}
        <div className="card space-y-2">
          <h3 className="text-sm font-semibold text-white">Pull (Registry)</h3>
          <input
            className="input w-full"
            placeholder="mariadb:11.4"
            value={pullRef}
            onChange={(e) => setPullRef(e.target.value)}
          />
          <input
            className="input w-full"
            placeholder="Registry URL o path (opcional)"
            value={pullRegistry}
            onChange={(e) => setPullRegistry(e.target.value)}
          />
          <button
            onClick={handlePull}
            disabled={!pullRef || loading === "pull"}
            className="btn-primary w-full"
          >
            {loading === "pull" ? "Descargando..." : "⬇ Pull"}
          </button>
        </div>

        {/* Import */}
        <div className="card space-y-2">
          <h3 className="text-sm font-semibold text-white">Import (Bundle local)</h3>
          <input
            className="input w-full"
            placeholder="ruta/al/bundle.tar.gz"
            value={importPath}
            onChange={(e) => setImportPath(e.target.value)}
          />
          <button
            onClick={handleImport}
            disabled={!importPath || loading === "import"}
            className="btn-primary w-full"
          >
            {loading === "import" ? "Importando..." : "📥 Import"}
          </button>
        </div>

        {/* Export */}
        <div className="card space-y-2">
          <h3 className="text-sm font-semibold text-white">Export</h3>
          <input
            className="input w-full"
            placeholder="mariadb:11.4"
            value={exportRef}
            onChange={(e) => setExportRef(e.target.value)}
          />
          <input
            className="input w-full"
            placeholder="salida.tar.gz"
            value={exportOutput}
            onChange={(e) => setExportOutput(e.target.value)}
          />
          <button
            onClick={handleExport}
            disabled={!exportRef || !exportOutput || loading === "export"}
            className="btn-primary w-full"
          >
            {loading === "export" ? "Exportando..." : "📤 Export"}
          </button>
        </div>
      </div>

      {/* Image list */}
      {images.length === 0 ? (
        <div className="card text-center py-12">
          <p className="text-dark-400">No hay imágenes disponibles</p>
          <p className="text-xs text-dark-500 mt-1">Importa un bundle o haz pull de un registry</p>
        </div>
      ) : (
        <div className="card overflow-hidden !p-0">
          <table className="w-full">
            <thead>
              <tr className="border-b border-dark-600 text-xs text-dark-400">
                <th className="text-left px-4 py-2">Referencia</th>
                <th className="text-left px-4 py-2">Motor</th>
                <th className="text-left px-4 py-2">Versión</th>
                <th className="text-left px-4 py-2">Arquitectura</th>
                <th className="text-left px-4 py-2">Layers</th>
                <th className="text-left px-4 py-2">Digest</th>
                <th className="text-right px-4 py-2">Acciones</th>
              </tr>
            </thead>
            <tbody>
              {images.map((img) => (
                <tr key={img.reference} className="border-b border-dark-700 hover:bg-dark-700/50">
                  <td className="px-4 py-2.5 text-sm text-white font-medium">
                    {engineIcons[img.database] || "💿"} {img.reference}
                  </td>
                  <td className="px-4 py-2.5 text-sm text-dark-200">{img.database}</td>
                  <td className="px-4 py-2.5 text-sm text-dark-300">{img.version}</td>
                  <td className="px-4 py-2.5 text-sm text-dark-300">{img.architecture}</td>
                  <td className="px-4 py-2.5 text-sm text-dark-300">{img.layer_count}</td>
                  <td className="px-4 py-2.5 text-xs text-dark-500 font-mono truncate max-w-[200px]">
                    {img.digest}
                  </td>
                  <td className="px-4 py-2.5 text-right">
                    <button
                      onClick={() => handleRemove(img.reference)}
                      disabled={loading === `remove-${img.reference}`}
                      className="text-xs px-2 py-1 rounded bg-red-500/20 text-red-400 hover:bg-red-500/30 disabled:opacity-50"
                    >
                      🗑 Rmi
                    </button>
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
