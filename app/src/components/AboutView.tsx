import { useCallback, useEffect, useState } from "react";
import { api, type AboutInfo, type UpdateInfo, type UpdateProgress } from "../api";
import mascot from "../assets/images/mascot-cleanup.png";
import { driveLabel, formatSize } from "../format";
import { reveal } from "../hooks";
import { Icon } from "./Icon";
import { Alert, Badge, IconButton } from "./ui";

/** Estado de las actualizaciones y el botón para buscar o instalar la versión nueva. */
function UpdateBox({ onError }: { onError: (message: string) => void }) {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [restarting, setRestarting] = useState(false);

  const check = useCallback(
    async (force: boolean) => {
      setChecking(true);
      try {
        setInfo(await api.updateStatus(force));
      } catch (e) {
        if (force) onError(String(e));
      } finally {
        setChecking(false);
      }
    },
    [onError],
  );

  useEffect(() => {
    void check(false);
  }, [check]);

  const install = async () => {
    setProgress({ done: 0, total: info?.size ?? 0 });
    const stop = await api.onUpdateProgress(setProgress).catch(() => null);
    try {
      await api.installUpdate();
      setRestarting(true);
    } catch (e) {
      onError(String(e));
      setProgress(null);
    } finally {
      stop?.();
    }
  };

  if (restarting) {
    return (
      <div className="update-box" role="status">
        <span className="spinner" aria-hidden="true" />
        <span>Reiniciando con la versión {info?.latest}…</span>
      </div>
    );
  }
  if (progress) {
    const pct = progress.total > 0 ? Math.min(100, (progress.done / progress.total) * 100) : 0;
    return (
      <div className="update-box column" role="status">
        <span>
          Descargando la versión {info?.latest}… {formatSize(progress.done)} de {formatSize(progress.total)}
        </span>
        <span className="meter update-meter" aria-hidden="true">
          <span style={{ width: `${pct}%` }} />
        </span>
      </div>
    );
  }
  return (
    <div className="update-box">
      <span className="update-text">
        {checking && !info && "Buscando actualizaciones…"}
        {info && !info.available && (
          <>
            <Icon name="check" className="update-ok" /> Estás al día
          </>
        )}
        {info?.available && (
          <>
            Hay una versión nueva: <strong>{info.latest}</strong> ({formatSize(info.size)}) ·{" "}
            <button type="button" className="link" onClick={() => api.openReleasePage().catch((e) => onError(String(e)))}>
              Novedades
            </button>
          </>
        )}
      </span>
      {info?.available ? (
        <button type="button" className="btn btn-primary btn-sm" onClick={() => void install()}>
          <Icon name="refresh" /> Actualizar a {info.latest}
        </button>
      ) : (
        <button type="button" className="btn btn-outline btn-sm" disabled={checking} onClick={() => void check(true)}>
          {checking ? <span className="spinner" aria-hidden="true" /> : <Icon name="refresh" />} Buscar actualizaciones
        </button>
      )}
    </div>
  );
}

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
          <UpdateBox onError={setError} />
          <div className="about-actions">
            <button type="button" className="btn btn-outline btn-sm" onClick={() => api.openRepo().catch((e) => setError(String(e)))}>
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
