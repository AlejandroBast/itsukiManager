const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

/** Tamaño en unidades binarias, como el Explorador: `320 GB`, `8,5 GB`. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = value >= 10 ? 0 : 1;
  return `${value.toLocaleString("es-ES", { maximumFractionDigits: digits })} ${UNITS[unit]}`;
}

/** Porcentaje con un decimal: `42,7%`. */
export function formatPercent(part: number, total: number, digits = 1): string {
  const value = total > 0 ? (part / total) * 100 : 0;
  return `${value.toLocaleString("es-ES", { minimumFractionDigits: digits, maximumFractionDigits: digits })}%`;
}

/** Cantidad con separador de miles: `596.169`. */
export function formatCount(n: number): string {
  return n.toLocaleString("es-ES");
}

/** Fecha corta a partir de segundos Unix: `12 mar 2025` (vacío si se desconoce). */
export function formatDate(unixSeconds: number): string {
  if (!unixSeconds) return "—";
  return new Date(unixSeconds * 1000).toLocaleDateString("es-ES", { day: "numeric", month: "short", year: "numeric" });
}

/** Duración legible: `850 ms`, `6,2 s`, `1 min 5 s`. */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toLocaleString("es-ES", { maximumFractionDigits: 1 })} s`;
  return `${Math.floor(s / 60)} min ${Math.round(s % 60)} s`;
}

/** Letra de la unidad sin la barra final: `C:`. */
export function driveLabel(root: string): string {
  return root.replace(/\\+$/, "");
}
