import { invoke } from "@tauri-apps/api/core";
import type { Container, ContainerLogs, ImageSummary, Volume, EngineInfo } from "./types";

export const api = {
  // Containers
  listContainers: () => invoke<Container[]>("list_containers"),
  inspectContainer: (name: string) => invoke<Container>("inspect_container", { name }),
  createContainer: (
    name: string,
    image: string,
    opts: { memory_mb?: number; cpu_quota?: number; pids_max?: number; ports?: string[] } = {}
  ) =>
    invoke<Container>("create_container", {
      name,
      image,
      memoryMb: opts.memory_mb ?? null,
      cpuQuota: opts.cpu_quota ?? null,
      pidsMax: opts.pids_max ?? null,
      ports: opts.ports ?? [],
    }),
  startContainer: (name: string) => invoke<void>("start_container", { name }),
  stopContainer: (name: string) => invoke<void>("stop_container", { name }),
  destroyContainer: (name: string) => invoke<void>("destroy_container", { name }),
  readLogs: (name: string) => invoke<ContainerLogs>("read_logs", { name }),

  // Images
  listImages: () => invoke<ImageSummary[]>("list_images"),
  importImage: (path: string) => invoke<string>("import_image", { path }),
  exportImage: (reference: string, output: string) =>
    invoke<void>("export_image", { reference, output }),
  pullImage: (reference: string, registry?: string) =>
    invoke<string>("pull_image", { reference, registry: registry ?? null }),
  removeImage: (reference: string) => invoke<void>("remove_image", { reference }),

  // Volumes
  listVolumes: () => invoke<Volume[]>("list_volumes"),
  createVolume: (name: string) => invoke<Volume>("create_volume", { name }),
  removeVolume: (name: string) => invoke<void>("remove_volume", { name }),
  backupVolume: (name: string, dest: string) =>
    invoke<string>("backup_volume", { name, dest }),

  // Config
  getDataRoot: () => invoke<string>("get_data_root"),
  getEngineInfo: () => invoke<EngineInfo[]>("get_engine_info"),
};
