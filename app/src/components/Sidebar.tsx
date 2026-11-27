import type { CSSProperties } from "react";
import mascot from "../assets/images/mascot-sidebar.png";
import watermark from "../assets/images/mascot-watermark.png";
import { Icon, type IconName } from "./Icon";

export type Section = "inicio" | "analizar" | "grandes" | "duplicados" | "limpieza" | "configuracion" | "acerca";

export const SECTIONS: { id: Section; label: string; icon: IconName }[] = [
  { id: "inicio", label: "Inicio", icon: "home" },
  { id: "analizar", label: "Analizar disco", icon: "search" },
  { id: "grandes", label: "Archivos grandes", icon: "layers" },
  { id: "duplicados", label: "Duplicados", icon: "files" },
  { id: "limpieza", label: "Limpieza", icon: "trash" },
  { id: "configuracion", label: "Configuración", icon: "settings" },
  { id: "acerca", label: "Acerca de", icon: "info" },
];

export function Sidebar({ section, onSelect }: { section: Section; onSelect: (s: Section) => void }) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <img src={mascot} alt="Itsuki con una carpeta" />
        <div className="brand-text">
          <div className="brand-name">
            Itsuki<span>Manager</span>
          </div>
          <div className="brand-sub">Gestor de almacenamiento</div>
        </div>
      </div>
      <nav className="nav" aria-label="Secciones">
        <span
          className="nav-highlight"
          aria-hidden="true"
          style={{ "--i": Math.max(0, SECTIONS.findIndex((s) => s.id === section)) } as CSSProperties}
        />
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            type="button"
            className={s.id === section ? "nav-item active" : "nav-item"}
            aria-current={s.id === section ? "page" : undefined}
            onClick={() => onSelect(s.id)}
          >
            <Icon name={s.icon} />
            {s.label}
          </button>
        ))}
      </nav>
      <span className="watermark-star" aria-hidden="true">
        <Icon name="star" />
      </span>
      <img className="watermark" src={watermark} alt="" />
    </aside>
  );
}
