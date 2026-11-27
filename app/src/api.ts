import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Unidad del sistema tal como la muestra la tarjeta de almacenamiento. */
export interface DiskInfo {
  root: string;
  name: string;
  kind: string;
  filesystem: string;
  total: number;
  used: number;
  free: number;
}

export type UsageKey = "apps" | "files" | "games" | "system" | "other";

export interface UsageSlice {
  key: UsageKey;
  label: string;
  bytes: number;
}

export interface FolderRow {
  name: string;
  path: string;
  bytes: number;
}

/** Datos de la pantalla de inicio, calculados por el núcleo de itsuki. */
export interface HomeSummary {
  root: string;
  used: number;
  usage: UsageSlice[];
  folders: FolderRow[];
  reclaimable: number;
  engine: string;
  elapsedMs: number;
}

export interface ScanProgress {
  files: number;
  bytes: number;
}

export const api = {
  systemDisk: () => invoke<DiskInfo | null>("system_disk"),
  homeSummary: (refresh: boolean) => invoke<HomeSummary>("home_summary", { refresh }),
  onScanProgress: (handler: (p: ScanProgress) => void): Promise<UnlistenFn> =>
    listen<ScanProgress>("scan-progress", (e) => handler(e.payload)),
};
