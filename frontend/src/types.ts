// Types matching tokedb-runtime data structures

export type ContainerState =
  | "created"
  | "starting"
  | "running"
  | "stopping"
  | "stopped"
  | "destroyed";

export interface ResourceLimits {
  memory_bytes: number | null;
  cpu_quota: number | null;
  pids_max: number | null;
}

export interface VolumeMount {
  name: string;
  mount_path: string;
}

export interface PortBinding {
  host_port: number;
  container_port: number;
  protocol: "tcp" | "udp";
}

export interface CommandSpec {
  program: string;
  args: string[];
}

export interface Container {
  id: string;
  name: string;
  image: string;
  command: CommandSpec;
  resources: ResourceLimits;
  volumes: VolumeMount[];
  ports: PortBinding[];
  state: ContainerState;
  created_at: number;
  pid: number | null;
}

export interface ContainerLogs {
  stdout: string;
  stderr: string;
}

export interface ImageSummary {
  reference: string;
  database: string;
  version: string;
  architecture: string;
  digest: string;
  layer_count: number;
}

export interface Volume {
  name: string;
  path: string;
}

export interface EngineInfo {
  name: string;
  engine: string;
  default_port: number;
  data_directory: string;
}

export type View = "dashboard" | "containers" | "images" | "volumes" | "settings";
