import type { DiskInfo } from "../api";
import { formatCount, formatSize } from "../format";
import { Icon } from "./Icon";

interface Props {
  disk: DiskInfo | null;
  busy: boolean;
  scannedFiles: number | null;
  onAnalyze: () => void;
}

/** Tarjeta de la unidad del sistema y botón "Analizar disco". */
export function DiskCard({ disk, busy, scannedFiles, onAnalyze }: Props) {
  const pct = disk && disk.total > 0 ? Math.round((disk.used / disk.total) * 100) : 0;
  return (
    <div className="disk-row rise" style={{ animationDelay: "60ms" }}>
      <section className="card disk-card" aria-label="Almacenamiento">
        <Icon name="drive" />
        <div>
          <div className="disk-head">
            <div>
              <div className="disk-name">{disk ? disk.name : "Disco local"}</div>
              <div className="disk-type">{disk ? `${disk.kind} · ${disk.filesystem}` : " "}</div>
            </div>
            <div>
              <div className="disk-amount">
                {disk ? formatSize(disk.used) : "—"}
                <span className="total">/ {disk ? formatSize(disk.total) : "—"}</span>
                <Icon name="chevron" />
              </div>
              <div className="disk-percent">{disk ? `${pct}%` : "—"}</div>
            </div>
          </div>
          <div
            className="progress"
            role="progressbar"
            aria-label="Espacio usado"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
          >
            <span style={{ width: `${pct}%` }} />
          </div>
        </div>
      </section>
      <div className="analyze-wrap">
        <button type="button" className="btn btn-primary analyze" onClick={onAnalyze} disabled={busy}>
          {busy ? <span className="spinner" aria-hidden="true" /> : <Icon name="play" />}
          {busy ? "Analizando…" : "Analizar disco"}
        </button>
        <svg className="sparkle" viewBox="0 0 40 40" aria-hidden="true">
          <g stroke="#FFD054" strokeWidth="3" strokeLinecap="round">
            <path d="M8 6v9" />
            <path d="M24 9l-6 8" />
            <path d="M34 24l-9 3" />
          </g>
        </svg>
        {busy && scannedFiles !== null && (
          <span className="sr-only" aria-live="polite">
            {formatCount(scannedFiles)} archivos analizados
          </span>
        )}
      </div>
    </div>
  );
}
