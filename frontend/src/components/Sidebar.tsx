import type { View } from "../types";

interface SidebarProps {
  current: View;
  onNavigate: (view: View) => void;
  containerCount: number;
  imageCount: number;
  volumeCount: number;
  runningCount: number;
}

const navItems: { view: View; icon: string; label: string }[] = [
  { view: "dashboard", icon: "📊", label: "Dashboard" },
  { view: "containers", icon: "📦", label: "Contenedores" },
  { view: "images", icon: "💿", label: "Imágenes" },
  { view: "volumes", icon: "💾", label: "Volúmenes" },
  { view: "settings", icon: "⚙️", label: "Configuración" },
];

export function Sidebar({
  current,
  onNavigate,
  containerCount,
  imageCount,
  volumeCount,
  runningCount,
}: SidebarProps) {
  return (
    <aside className="w-56 bg-dark-800 border-r border-dark-600 flex flex-col">
      {/* Logo */}
      <div className="p-4 border-b border-dark-600">
        <div className="flex items-center gap-2">
          <span className="text-xl">🗃️</span>
          <div>
            <h1 className="text-sm font-bold text-white">tokedb</h1>
            <p className="text-[10px] text-dark-400">Manager</p>
          </div>
        </div>
      </div>

      {/* Nav */}
      <nav className="flex-1 p-2 space-y-0.5">
        {navItems.map((item) => {
          const isActive = current === item.view;
          let count = 0;
          if (item.view === "containers") count = containerCount;
          if (item.view === "images") count = imageCount;
          if (item.view === "volumes") count = volumeCount;

          return (
            <button
              key={item.view}
              onClick={() => onNavigate(item.view)}
              className={`w-full flex items-center gap-2.5 px-3 py-2 rounded-lg text-sm transition-all ${
                isActive
                  ? "bg-primary-500/20 text-primary-400 font-medium"
                  : "text-dark-300 hover:bg-dark-700 hover:text-white"
              }`}
            >
              <span className="text-base">{item.icon}</span>
              <span className="flex-1 text-left">{item.label}</span>
              {count > 0 && (
                <span className="text-[10px] bg-dark-600 text-dark-300 px-1.5 py-0.5 rounded-full">
                  {count}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      {/* Status */}
      <div className="p-3 border-t border-dark-600">
        <div className="flex items-center gap-2 text-xs text-dark-400">
          <span
            className={`w-2 h-2 rounded-full ${
              runningCount > 0 ? "bg-green-500 animate-pulse" : "bg-dark-500"
            }`}
          />
          <span>
            {runningCount > 0
              ? `${runningCount} contenedor(es) activo(s)`
              : "Sin contenedores activos"}
          </span>
        </div>
      </div>
    </aside>
  );
}
