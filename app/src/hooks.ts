import { useCallback, useState } from "react";
import { api, isCancelled, type ScanProgress } from "./api";

export type JobPhase = "scan" | "dupes";

/**
 * Ejecuta trabajos largos (escanear una unidad, comparar archivos) mostrando su
 * progreso. Cancelar no cuenta como error.
 */
export function useJob() {
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [phase, setPhase] = useState<JobPhase | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(async <T>(work: () => Promise<T>): Promise<T | null> => {
    setBusy(true);
    setError(null);
    const stops = await Promise.all([
      api.onScanProgress((p) => {
        setPhase("scan");
        setProgress(p);
      }),
      api.onDupesProgress((p) => {
        setPhase("dupes");
        setProgress(p);
      }),
    ]).catch(() => []);
    try {
      return await work();
    } catch (e) {
      if (!isCancelled(e)) setError(String(e));
      return null;
    } finally {
      stops.forEach((stop) => stop());
      setProgress(null);
      setPhase(null);
      setBusy(false);
    }
  }, []);

  return { busy, progress, phase, error, setError, run };
}

/** Abre el Explorador en la ruta; si falla, informa con `onError`. */
export function reveal(path: string, onError: (message: string) => void) {
  api.revealPath(path).catch((e) => onError(String(e)));
}
