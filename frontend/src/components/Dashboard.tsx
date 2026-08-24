import type { Container, ImageSummary, Volume } from "../types";

interface Props {
  containers: Container[];
  images: ImageSummary[];
  volumes: Volume[];
  onSelectView: (view: string) => void;
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

export function Dashboard({ containers, images, volumes, onSelectView }: Props) {
  const running = containers.filter((c) => c.state === "running").length;
  const stopped = containers.filter((c) => c.state === "stopped").length;
  const totalPorts = containers.reduce((acc, c) => acc + c.ports.length, 0);

  return (
    <div className="p-6 space-y-6 animate-fade-in">
      <div>
        <h2 className="text-xl font-bold text-white mb-1">Dashboard</h2>
        <p className="text-sm text-dark-400">Vista general del runtime tokedb</p>
      </div>

      {/* Stats cards */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        <StatCard
          icon="📦"
          label="Contenedores"
          value={containers.length}
          detail={`${running} activos`}
          onClick={() => onSelectView("containers")}
        />
        <StatCard
          icon="💿"
          label="Imágenes"
          value={images.length}
          detail={`${new Set(images.map((i) => i.database)).size} motores`}
          onClick={() => onSelectView("images")}
        />
        <StatCard
          icon="💾"
          label="Volúmenes"
          value={volumes.length}
          detail="datos persistentes"
          onClick={() => onSelectView("volumes")}
        />
        <StatCard
          icon="🔌"
          label="Puertos"
          value={totalPorts}
          detail="publicados"
        />
      </div>

      {/* Two columns */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Containers summary */}
        <div className="card">
          <h3 className="text-sm font-semibold text-white mb-3">Contenedores recientes</h3>
          {containers.length === 0 ? (
            <p className="text-xs text-dark-400 py-4 text-center">
              No hay contenedores. Crea uno desde la pestaña de Contenedores.
            </p>
          ) : (
            <div className="space-y-2">
              {containers.slice(0, 5).map((c) => (
                <div key={c.id} className="flex items-center justify-between py-1.5 px-2 rounded bg-dark-700/50">
                  <div className="flex items-center gap-2">
                    <span>{engineIcons[c.image.split(":")[0]] || "📦"}</span>
                    <div>
                      <span className="text-sm text-white">{c.name}</span>
                      <span className="text-xs text-dark-400 ml-2">{c.image}</span>
                    </div>
                  </div>
                  <span className={stateColors[c.state]}>{c.state}</span>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Images summary */}
        <div className="card">
          <h3 className="text-sm font-semibold text-white mb-3">Imágenes disponibles</h3>
          {images.length === 0 ? (
            <p className="text-xs text-dark-400 py-4 text-center">
              No hay imágenes. Importa o descarga una desde la pestaña de Imágenes.
            </p>
          ) : (
            <div className="space-y-2">
              {images.map((img) => (
                <div key={img.reference} className="flex items-center justify-between py-1.5 px-2 rounded bg-dark-700/50">
                  <div className="flex items-center gap-2">
                    <span>{engineIcons[img.database] || "💿"}</span>
                    <div>
                      <span className="text-sm text-white">{img.reference}</span>
                      <span className="text-xs text-dark-400 ml-2">{img.architecture}</span>
                    </div>
                  </div>
                  <span className="badge-blue">{img.layer_count} layer(s)</span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function StatCard({
  icon,
  label,
  value,
  detail,
  onClick,
}: {
  icon: string;
  label: string;
  value: number;
  detail: string;
  onClick?: () => void;
}) {
  return (
    <div
      className={`card group ${onClick ? "cursor-pointer hover:border-dark-500" : ""}`}
      onClick={onClick}
    >
      <div className="flex items-start justify-between">
        <div>
          <p className="text-xs text-dark-400 mb-1">{label}</p>
          <p className="text-2xl font-bold text-white">{value}</p>
          <p className="text-xs text-dark-400 mt-1">{detail}</p>
        </div>
        <span className="text-2xl opacity-60 group-hover:opacity-100 transition-opacity">
          {icon}
        </span>
      </div>
    </div>
  );
}
