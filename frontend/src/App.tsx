import { useEffect, useState, useCallback } from "react";
import { Sidebar } from "./components/Sidebar";
import { Dashboard } from "./components/Dashboard";
import { ContainersView } from "./components/ContainersView";
import { ImagesView } from "./components/ImagesView";
import { VolumesView } from "./components/VolumesView";
import { SettingsView } from "./components/SettingsView";
import { api } from "./api";
import type { Container, ImageSummary, Volume, View } from "./types";

function App() {
  const [view, setView] = useState<View>("dashboard");
  const [containers, setContainers] = useState<Container[]>([]);
  const [images, setImages] = useState<ImageSummary[]>([]);
  const [volumes, setVolumes] = useState<Volume[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [c, i, v] = await Promise.all([
        api.listContainers().catch(() => []),
        api.listImages().catch(() => []),
        api.listVolumes().catch(() => []),
      ]);
      setContainers(c);
      setImages(i);
      setVolumes(v);
      setError(null);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 5000);
    return () => clearInterval(interval);
  }, [refresh]);

  const runningCount = containers.filter((c) => c.state === "running").length;

  return (
    <div className="h-screen flex bg-dark-900">
      <Sidebar
        current={view}
        onNavigate={setView}
        containerCount={containers.length}
        imageCount={images.length}
        volumeCount={volumes.length}
        runningCount={runningCount}
      />

      <main className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-center">
              <div className="text-4xl mb-3">🗃️</div>
              <p className="text-dark-400 text-sm">Cargando runtime...</p>
            </div>
          </div>
        ) : error ? (
          <div className="p-6">
            <div className="bg-red-500/10 border border-red-500/30 rounded-lg p-4">
              <p className="text-sm text-red-400 font-medium">Error de conexión</p>
              <p className="text-xs text-red-400/70 mt-1">{error}</p>
              <button onClick={refresh} className="btn-ghost mt-3 text-xs">
                Reintentar
              </button>
            </div>
          </div>
        ) : (
          <>
            {view === "dashboard" && (
              <Dashboard
                containers={containers}
                images={images}
                volumes={volumes}
                onSelectView={(v) => setView(v as View)}
              />
            )}
            {view === "containers" && (
              <ContainersView containers={containers} images={images} onRefresh={refresh} />
            )}
            {view === "images" && (
              <ImagesView images={images} onRefresh={refresh} />
            )}
            {view === "volumes" && (
              <VolumesView volumes={volumes} onRefresh={refresh} />
            )}
            {view === "settings" && <SettingsView />}
          </>
        )}
      </main>
    </div>
  );
}

export default App;
