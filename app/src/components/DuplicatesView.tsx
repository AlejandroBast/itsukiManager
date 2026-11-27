import { useEffect, useState } from "react";
import { api, type DiskInfo, type DupFile, type DupGroup, type DupView } from "../api";
import { driveLabel, formatCount, formatDate, formatDuration, formatSize } from "../format";
import { reveal, useJob } from "../hooks";
import { Icon } from "./Icon";
import { Alert, Badge, BusyState, EmptyState, IconButton, Modal, PageHeader, Segmented, SelectBox, type Option } from "./ui";

const MB = 1024 ** 2;
export const DUP_MINS: Option<string>[] = [
  { value: String(100 * 1024), label: "100 KB o más" },
  { value: String(MB), label: "1 MB o más" },
  { value: String(10 * MB), label: "10 MB o más" },
  { value: String(100 * MB), label: "100 MB o más" },
];

const PERSONAL_HINT = "Escritorio, Documentos, Descargas, Imágenes, Vídeos, Música y OneDrive";

/** Quita de los resultados los archivos enviados a la Papelera. */
function withoutFiles(view: DupView, paths: Set<string>): DupView {
  const groups: DupGroup[] = [];
  for (const g of view.groups) {
    const files = g.files.filter((f) => !paths.has(f.path));
    if (files.length < 2) continue;
    const perCopy = g.files.length > 1 ? g.wasted / (g.files.length - 1) : 0;
    groups.push({ ...g, files, wasted: Math.round(perCopy * (files.length - 1)) });
  }
  const removedGroups = view.groups.length - groups.length;
  return {
    ...view,
    groups,
    totalGroups: view.totalGroups - removedGroups,
    wasted: groups.reduce((acc, g) => acc + g.wasted, 0),
  };
}

interface Pending {
  files: DupFile[];
  keep?: DupFile;
  bytes: number;
}

interface Props {
  drives: DiskInfo[];
  defaultScope: string;
  defaultMin: number;
  onChanged: () => void;
}

/** «Duplicados»: archivos con el mismo contenido (no solo el mismo nombre). */
export function DuplicatesView({ drives, defaultScope, defaultMin, onChanged }: Props) {
  const [scope, setScope] = useState(defaultScope);
  const [min, setMin] = useState(String(defaultMin));

  // Al cambiar los valores por defecto en Configuración (si la unidad ya no está, carpetas personales).
  useEffect(() => {
    setScope(defaultScope === "personal" || drives.some((d) => d.root === defaultScope) ? defaultScope : "personal");
  }, [defaultScope, drives]);
  useEffect(() => setMin(String(defaultMin)), [defaultMin]);
  const [data, setData] = useState<DupView | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const { busy, progress, phase, error, setError, run } = useJob();

  const search = async () => {
    setNotice(null);
    const view = await run(() => api.findDupes(scope, Number(min)));
    if (view) setData(view);
  };

  const recycle = async (p: Pending) => {
    setPending(null);
    try {
      await api.recyclePaths(p.files.map((f) => f.path));
      setData((d) => d && withoutFiles(d, new Set(p.files.map((f) => f.path))));
      setNotice(
        p.files.length === 1
          ? `«${p.files[0].name}» se envió a la Papelera.`
          : `${formatCount(p.files.length)} copias se enviaron a la Papelera (${formatSize(p.bytes)}).`,
      );
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  };

  const scopes: Option<string>[] = [
    { value: "personal", label: "Carpetas personales", hint: PERSONAL_HINT },
    ...drives.map((d) => ({ value: d.root, label: driveLabel(d.root), hint: `${d.name}: toda la unidad (sin Windows ni programas)` })),
  ];
  const scopeLabel = scope === "personal" ? "tus carpetas personales" : driveLabel(scope);

  const progressText = () => {
    if (!progress) return "Preparando";
    if (phase === "dupes") return `${formatCount(progress.files)} archivos comparados · ${formatSize(progress.bytes)} leídos`;
    return `${formatCount(progress.files)} archivos encontrados`;
  };

  return (
    <section className="page rise" aria-label="Duplicados">
      <PageHeader title="Duplicados" subtitle="Mismo contenido: deja una sola copia.">
        <Segmented label="Dónde buscar" options={scopes} value={scope} disabled={busy} onChange={setScope} />
        <SelectBox label="Tamaño mínimo" options={DUP_MINS} value={min} disabled={busy} onChange={setMin} />
        <button type="button" className="btn btn-primary btn-sm" disabled={busy} onClick={() => void search()}>
          <Icon name="search" /> {data ? "Buscar de nuevo" : "Buscar duplicados"}
        </button>
      </PageHeader>

      <div className="card page-card">
        {data && !busy && (
          <div className="page-meta">
            <span>
              <strong>{formatCount(data.totalGroups)}</strong> grupos de duplicados ·{" "}
              <strong>{formatSize(data.wasted)}</strong> recuperables dejando una copia de cada uno
            </span>
            <span>
              {formatCount(data.readFiles)} archivos comparados en {formatDuration(data.elapsedMs)}
            </span>
            {data.totalGroups > data.groups.length && <span>Mostrando los {formatCount(data.groups.length)} que más espacio liberan</span>}
            {data.errors > 0 && <span>{formatCount(data.errors)} no se pudieron leer (en uso o sin permiso)</span>}
          </div>
        )}
        {error && (
          <Alert tone="error" onClose={() => setError(null)}>
            {error}
          </Alert>
        )}
        {notice && (
          <Alert tone="ok" onClose={() => setNotice(null)}>
            {notice}
          </Alert>
        )}

        {busy && (
          <BusyState
            title={phase === "dupes" ? "Comparando el contenido…" : `Leyendo ${scopeLabel}…`}
            detail={progressText()}
            onCancel={() => void api.cancelJob()}
          />
        )}
        {!busy && !data && (
          <EmptyState
            title="Busca archivos repetidos"
            text={`Se compara el contenido de los archivos de ${scopeLabel}, así que dos copias con distinto nombre también se detectan. Se omiten Windows, programas, juegos y AppData.`}
          >
            <button type="button" className="btn btn-primary btn-md" onClick={() => void search()}>
              <Icon name="search" /> Buscar duplicados
            </button>
          </EmptyState>
        )}
        {!busy && data && data.groups.length === 0 && (
          <EmptyState title="Sin duplicados" text={`No hay archivos repetidos de ${DUP_MINS.find((o) => o.value === min)?.label ?? ""} en ${scopeLabel}.`} />
        )}
        {!busy && data && data.groups.length > 0 && (
          <div className="page-scroll">
            {data.groups.map((g) => {
              const [oldest, ...copies] = g.files;
              return (
                <div key={g.files.map((f) => f.path).join("|")} className="dup-group">
                  <div className="dup-head">
                    <span className="dup-title">
                      {formatCount(g.files.length)} copias · {formatSize(g.size)} cada una
                    </span>
                    <Badge tone="info">Recuperas {formatSize(g.wasted)}</Badge>
                    <button
                      type="button"
                      className="btn btn-outline btn-xs"
                      onClick={() => setPending({ files: copies, keep: oldest, bytes: g.wasted })}
                    >
                      Dejar solo la más antigua
                    </button>
                  </div>
                  {g.files.map((f, i) => (
                    <div key={f.path} className="row cols-dup">
                      <div className="cell-file">
                        <span className="name" title={f.name}>
                          {f.name}
                        </span>
                        <span className="sub" title={f.folder}>
                          {f.folder}
                        </span>
                      </div>
                      <span className="muted">{formatDate(f.modified)}</span>
                      <span>{i === 0 && <Badge tone="safe">Más antigua</Badge>}</span>
                      <span className="row-actions">
                        <IconButton icon="open" label="Mostrar en el Explorador" onClick={() => reveal(f.path, setError)} />
                        <IconButton
                          icon="trash"
                          label="Enviar a la Papelera"
                          danger
                          onClick={() => setPending({ files: [f], bytes: g.wasted / Math.max(1, g.files.length - 1) })}
                        />
                      </span>
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {pending && (
        <Modal
          title="¿Enviar a la Papelera?"
          onClose={() => setPending(null)}
          actions={
            <>
              <button type="button" className="btn btn-outline btn-sm" autoFocus onClick={() => setPending(null)}>
                Cancelar
              </button>
              <button type="button" className="btn btn-danger btn-sm" onClick={() => void recycle(pending)}>
                <Icon name="trash" /> Enviar a la Papelera
              </button>
            </>
          }
        >
          {pending.keep ? (
            <p>
              Se enviarán <strong>{formatCount(pending.files.length)}</strong> copias a la Papelera ({formatSize(pending.bytes)}) y se
              conserva la más antigua:
            </p>
          ) : (
            <p>
              Esta copia de <strong>«{pending.files[0].name}»</strong> irá a la Papelera. Las demás copias se conservan.
            </p>
          )}
          <p className="modal-path">{pending.keep ? pending.keep.path : pending.files[0].path}</p>
          <p>Podrás restaurar lo enviado desde la Papelera; el espacio se libera al vaciarla.</p>
        </Modal>
      )}
    </section>
  );
}
