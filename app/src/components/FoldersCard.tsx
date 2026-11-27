import type { CSSProperties } from "react";
import { useState } from "react";
import type { FolderRow } from "../api";
import { formatSize } from "../format";
import { Icon } from "./Icon";

interface Props {
  folders: FolderRow[] | null;
  onSeeAll: () => void;
}

/** "Carpetas que más espacio ocupan": las carpetas más grandes de la unidad. */
export function FoldersCard({ folders, onSeeAll }: Props) {
  const [selected, setSelected] = useState(0);
  return (
    <section className="card folders rise" style={{ animationDelay: "180ms" }} aria-label="Carpetas que más espacio ocupan">
      <div className="folders-head">
        <h2 className="card-title">Carpetas que más espacio ocupan</h2>
        <button type="button" className="link" onClick={onSeeAll}>
          Ver todo <Icon name="arrow" />
        </button>
      </div>
      <ul className="folder-list">
        {folders
          ? folders.map((f, i) => (
              <li key={f.path} className="stagger" style={{ "--i": i } as CSSProperties}>
                <button
                  type="button"
                  className={i === selected ? "folder-row selected" : "folder-row"}
                  title={f.path}
                  onClick={() => setSelected(i)}
                >
                  <Icon name="folder" />
                  <span className="name">{f.name}</span>
                  <span className="size">{formatSize(f.bytes)}</span>
                </button>
              </li>
            ))
          : Array.from({ length: 5 }, (_, i) => (
              <li key={i}>
                <div className="folder-row" aria-hidden="true">
                  <Icon name="folder" />
                  <span className="skeleton" style={{ width: `${12 - i}rem`, height: "1.1rem" }} />
                  <span className="skeleton" style={{ width: "4rem", height: "1.1rem", marginLeft: "auto" }} />
                </div>
              </li>
            ))}
      </ul>
    </section>
  );
}
