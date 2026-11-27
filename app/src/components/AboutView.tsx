import { useEffect, useState } from "react";
import { api, type AboutInfo } from "../api";
import mascot from "../assets/images/mascot-cleanup.png";
import { driveLabel, formatSize } from "../format";
import { reveal } from "../hooks";
import { Icon } from "./Icon";
import { Alert, Badge, IconButton } from "./ui";

/** «Acerca de»: versión, enlaces, créditos e información del sistema. */
export function AboutView() {
  const [info, setInfo] = useState<AboutInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.aboutInfo().then(setInfo, (e) => setError(String(e)));
  }, []);

  return (
    <section className="page rise about" aria-label="Acerca de">
      {error && (
        <Alert tone="error" onClose={() => setError(null)}>
          {error}
        </Alert>
      )}
      <div className="about-grid">
        <div className="card about-hero">
          <img src={mascot} alt="Itsuki, la mascota de ItsukiManager" />
          <div className="brand-name about-name">
            Itsuki<span>Manager</span>
          </div>
          <div className="about-badges">
            <Badge tone="info">Versión {info?.version ?? "…"}</Badge>
            <Badge tone="safe">Código abierto</Badge>
          </div>
          <p>
            Gestor de almacenamiento para Windows: encuentra lo que ocupa tu disco, los archivos repetidos y la basura que se puede
            limpiar con seguridad.
          </p>
          <div className="about-actions">
            <button type="button" className="btn btn-primary btn-md" onClick={() => api.openRepo().catch((e) => setError(String(e)))}>
              <Icon name="open" /> Ver en GitHub
            </button>
          </div>
          <dl className="about-facts">
            <div>
              <dt>Autor</dt>
              <dd>AlejandroBast</dd>
            </div>
            <div>
              <dt>Hecho con</dt>
              <dd>Rust, Tauri y React</dd>
            </div>
          </dl>
        </div>

        <div className="about-side">
          <div className="card about-card">
            <h2 className="card-title">Sistema</h2>
            <dl className="about-list">
              <div>
                <dt>Permisos</dt>
                <dd>
                  {info && (
                    <Badge tone={info.elevated ? "safe" : "warn"}>{info.elevated ? "Administrador" : "Usuario normal"}</Badge>
                  )}
                  <span className="muted">
                    {info?.elevated
                      ? "Se usa la MFT en unidades NTFS (lo más rápido)."
                      : "Ábrela como administrador para escanear con la MFT y limpiar Windows Update."}
                  </span>
                </dd>
              </div>
              <div>
                <dt>Procesador</dt>
                <dd>{info ? `${info.threads} hilos para escanear en paralelo` : "…"}</dd>
              </div>
              <div>
                <dt>Datos</dt>
                <dd className="about-path">
                  <span title={info?.dataDir}>{info?.dataDir ?? "…"}</span>
                  {info && <IconButton icon="open" label="Abrir la carpeta de datos" onClick={() => reveal(info.dataDir, setError)} />}
                </dd>
              </div>
            </dl>
            <h3 className="about-sub">Unidades</h3>
            <div className="about-drives">
              {info?.drives.map((d) => (
                <div key={d.root} className="about-drive">
                  <span className="drive-letter">
                    <Icon name="drive" />
                    {driveLabel(d.root)}
                  </span>
                  <span className="name">{d.name}</span>
                  <span className="muted">
                    {d.kind} · {d.filesystem} · {formatSize(d.total)}
                  </span>
                  <Badge tone={d.engine === "MFT" ? "safe" : "info"} title="Motor que usa el escaneo automático de la unidad entera">
                    {d.engine}
                  </Badge>
                </div>
              ))}
            </div>
          </div>

          <div className="card about-card">
            <h2 className="card-title">Créditos</h2>
            <p className="about-text">
              ItsukiManager es de código abierto: su código está en GitHub y cualquiera puede revisarlo, usarlo y mejorarlo.
            </p>
            <p className="about-text">
              La mascota está inspirada en Itsuki Nakano, de <em>The Quintessential Quintuplets</em> (Negi Haruba).
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}
