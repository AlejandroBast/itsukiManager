import { useCallback, useEffect, useRef, useState } from "react";

function prefersReducedMotion(): boolean {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** Cuenta desde el valor anterior (0 al empezar) hasta `target`, frenando al final. `null` = aún sin dato. */
export function useCountUp(target: number | null, duration = 800): number {
  const [value, setValue] = useState(0);
  const current = useRef(0);
  useEffect(() => {
    if (target === null) return;
    if (prefersReducedMotion()) {
      current.current = target;
      setValue(target);
      return;
    }
    const from = current.current;
    const start = performance.now();
    let frame = 0;
    const tick = (now: number) => {
      const t = Math.min(1, (now - start) / duration);
      current.current = from + (target - from) * (1 - (1 - t) ** 3);
      setValue(current.current);
      if (t < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [target, duration]);
  return value;
}
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
