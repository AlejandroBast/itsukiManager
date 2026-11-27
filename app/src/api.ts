import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Unidad tal como la muestran la tarjeta de almacenamiento y los selectores. */
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

/** Datos del escaneo que acompañan a los resultados. */
export interface ScanInfo {
  root: string;
  used: number;
  total: number | null;
  files: number;
  /** Carpetas que no se pudieron leer (sin permisos de administrador). */
  denied: number;
  engine: string;
  elapsedMs: number;
}

/** Carpeta grande. `parent` es la carpeta grande que la contiene (0 = está en la raíz). */
export interface FolderItem {
  id: number;
  parent: number;
  name: string;
  path: string;
  bytes: number;
  files: number;
}

export interface FoldersView {
  scan: ScanInfo;
  folders: FolderItem[];
}

export interface FileItem {
  name: string;
  folder: string;
  path: string;
  bytes: number;
  /** Segundos Unix (0 = desconocida). */
  modified: number;
  kind: string;
  hidden: boolean;
}

export interface FilesView {
  scan: ScanInfo;
  files: FileItem[];
}

export interface DupFile {
  name: string;
  folder: string;
  path: string;
  modified: number;
}

/** Copias idénticas, de la más antigua a la más reciente. */
export interface DupGroup {
  size: number;
  wasted: number;
  files: DupFile[];
}

export interface DupView {
  groups: DupGroup[];
  totalGroups: number;
  wasted: number;
  readFiles: number;
  readBytes: number;
  errors: number;
  elapsedMs: number;
}

export type Safety = "safe" | "regenerable" | "review" | "advice";

export interface JunkItem {
  id: number;
  label: string;
  path: string;
  /** Ubicaciones adicionales del mismo hallazgo. */
  more: number;
  bytes: number;
  files: number;
  safety: Safety;
  needsAdmin: boolean;
  openApp: string | null;
  advice: string | null;
}

export interface JunkCategory {
  id: string;
  label: string;
  safety: Safety;
  bytes: number;
  files: number;
  items: JunkItem[];
}

export interface JunkView {
  root: string;
  categories: JunkCategory[];
  safe: number;
  regenerable: number;
  review: number;
  elevated: boolean;
}

export interface CleanResult {
  bytes: number;
  files: number;
  skipped: number;
  errors: string[];
  skippedOpen: { app: string; bytes: number }[];
  toRecycleBin: boolean;
}

export interface CleanProgress {
  done: number;
  total: number;
  bytes: number;
}

/** Error que devuelve el núcleo cuando el usuario cancela. */
export const CANCELLED = "cancelado";

export function isCancelled(e: unknown): boolean {
  return String(e) === CANCELLED;
}

function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => handler(e.payload));
}

export const api = {
  systemDisk: () => invoke<DiskInfo | null>("system_disk"),
  listDrives: () => invoke<DiskInfo[]>("list_drives"),
  homeSummary: (refresh: boolean) => invoke<HomeSummary>("home_summary", { refresh }),
  largeFolders: (root: string, min: number, refresh: boolean) =>
    invoke<FoldersView>("large_folders", { root, min, refresh }),
  largeFiles: (root: string, top: number, refresh: boolean) => invoke<FilesView>("large_files", { root, top, refresh }),
  findDupes: (scope: string, min: number) => invoke<DupView>("find_dupes", { scope, min }),
  junkScan: (refresh: boolean) => invoke<JunkView>("junk_scan", { refresh }),
  junkClean: (ids: number[], toRecycleBin: boolean, includeOpen: boolean) =>
    invoke<CleanResult>("junk_clean", { ids, toRecycleBin, includeOpen }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  recyclePaths: (paths: string[]) => invoke<void>("recycle_paths", { paths }),
  cancelJob: () => invoke<void>("cancel_job"),
  onScanProgress: (handler: (p: ScanProgress) => void) => on<ScanProgress>("scan-progress", handler),
  onDupesProgress: (handler: (p: ScanProgress) => void) => on<ScanProgress>("dupes-progress", handler),
  onCleanProgress: (handler: (p: CleanProgress) => void) => on<CleanProgress>("clean-progress", handler),
};
