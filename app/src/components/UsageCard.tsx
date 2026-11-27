import type { CSSProperties } from "react";
import type { HomeSummary, UsageKey } from "../api";
import { useCountUp } from "../hooks";
import { formatPercent, formatSize } from "../format";

/** Colores de la gráfica, en el orden de la leyenda del diseño. */
const COLORS: Record<UsageKey, string> = {
  apps: "var(--primary)",
  files: "var(--accent)",
  games: "var(--violet)",
  system: "var(--blue)",
  other: "var(--gray)",
};

/** Radio y grosor del anillo en unidades del SVG (diámetro exterior 211). */
const R = 87;
const CIRC = 2 * Math.PI * R;
const GAP = 1.2;

export function UsageCard({ summary }: { summary: HomeSummary | null }) {
  const used = useCountUp(summary?.used ?? null);
  const total = summary ? summary.usage.reduce((s, u) => s + u.bytes, 0) : 0;
  let offset = 0;
  let arcsCount = 0;
  const arcs = (summary?.usage ?? [])
    .filter((u) => u.bytes > 0)
    .map((u) => {
      const len = total > 0 ? (u.bytes / total) * CIRC : 0;
      const arc = { key: u.key, dash: Math.max(len - GAP, 0.5), offset, i: arcsCount++ };
      offset += len;
      return arc;
    });

  return (
    <section className="card usage rise" style={{ animationDelay: "120ms" }} aria-label="Uso del almacenamiento">
      <h2 className="card-title">Uso del almacenamiento</h2>
      <div className="usage-body">
        <div className="donut">
          <svg viewBox="0 0 211 211" aria-hidden="true">
            <circle cx="105.5" cy="105.5" r={R} stroke="#43262f" />
            {arcs.map((a) => (
              <circle
                key={a.key}
                className="arc"
                style={{ "--i": a.i, "--circ": CIRC } as CSSProperties}
                cx="105.5"
                cy="105.5"
                r={R}
                stroke={COLORS[a.key]}
                strokeDasharray={`${a.dash} ${CIRC - a.dash}`}
                strokeDashoffset={-a.offset}
              />
            ))}
          </svg>
          <div className="donut-center">
            {summary ? (
              <>
                <strong>{formatSize(used)}</strong>
                <span>usados</span>
              </>
            ) : (
              <span className="skeleton" style={{ width: "6rem", height: "1.4rem" }} />
            )}
          </div>
        </div>
        <ul className="legend">
          {summary
            ? summary.usage.map((u, i) => (
                <li key={u.key} className="stagger" style={{ "--i": i } as CSSProperties}>
                  <span className="swatch" style={{ background: COLORS[u.key] }} />
                  <span>{u.label}</span>
                  <span className="value">
                    {formatSize(u.bytes)} ({formatPercent(u.bytes, total)})
                  </span>
                </li>
              ))
            : PLACEHOLDER.map((u) => (
                <li key={u.key}>
                  <span className="swatch" style={{ background: COLORS[u.key] }} />
                  <span>{u.label}</span>
                  <span className="skeleton" style={{ width: "7.5rem", height: "1rem" }} />
                </li>
              ))}
        </ul>
      </div>
    </section>
  );
}

const PLACEHOLDER: { key: UsageKey; label: string }[] = [
  { key: "apps", label: "Aplicaciones" },
  { key: "files", label: "Archivos" },
  { key: "games", label: "Juegos" },
  { key: "system", label: "Sistema" },
  { key: "other", label: "Otros" },
];
