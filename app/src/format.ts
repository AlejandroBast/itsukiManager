const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

/** Tamaño en unidades binarias, como el Explorador: `320 GB`, `8,5 GB`. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
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
